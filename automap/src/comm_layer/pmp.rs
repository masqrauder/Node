// Copyright (c) 2019-2021, MASQ (https://masq.ai) and/or its affiliates. All rights reserved.

use crate::comm_layer::pcp_pmp_common::{find_routers, make_local_socket_address, ChangeHandlerConfig, FreePortFactory, FreePortFactoryReal, UdpSocketFactory, UdpSocketFactoryReal, UdpSocketWrapper, CHANGE_HANDLER_PORT, ROUTER_PORT, READ_TIMEOUT_MILLIS};
use crate::comm_layer::{AutomapError, AutomapErrorCause, Transactor, HousekeepingThreadCommand};
use crate::control_layer::automap_control::{AutomapChange, ChangeHandler};
use crate::protocols::pmp::get_packet::GetOpcodeData;
use crate::protocols::pmp::map_packet::MapOpcodeData;
use crate::protocols::pmp::pmp_packet::{Opcode, PmpPacket, ResultCode};
use crate::protocols::utils::{Direction, Packet};
use crossbeam_channel::{unbounded, Receiver, Sender};
use masq_lib::logger::Logger;
use masq_lib::utils::AutomapProtocol;
use masq_lib::{debug, error, info, warning};
use pretty_hex::PrettyHex;
use std::any::Any;
use std::cell::RefCell;
use std::convert::TryFrom;
use std::io::ErrorKind;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::ops::Deref;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

struct Factories {
    socket_factory: Box<dyn UdpSocketFactory>,
    free_port_factory: Box<dyn FreePortFactory>,
}

impl Default for Factories {
    fn default() -> Self {
        Self {
            socket_factory: Box::new(UdpSocketFactoryReal::new()),
            free_port_factory: Box::new(FreePortFactoryReal::new()),
        }
    }
}

pub struct PmpTransactor {
    mapping_adder_arc: Arc<Mutex<Box<dyn MappingAdder>>>,
    factories_arc: Arc<Mutex<Factories>>,
    router_port: u16,
    listen_port: u16,
    change_handler_config_opt: RefCell<Option<ChangeHandlerConfig>>,
    housekeeper_commander_opt: Option<Sender<HousekeepingThreadCommand>>,
    read_timeout_millis: u64,
    logger: Logger,
}

impl Transactor for PmpTransactor {
    fn find_routers(&self) -> Result<Vec<IpAddr>, AutomapError> {
        find_routers()
    }

    fn get_public_ip(&self, router_ip: IpAddr) -> Result<IpAddr, AutomapError> {
        let request = PmpPacket {
            direction: Direction::Request,
            opcode: Opcode::Get,
            result_code_opt: None,
            opcode_data: Box::new(GetOpcodeData {
                epoch_opt: None,
                external_ip_address_opt: None,
            }),
        };
        let response = Self::transact(&self.factories_arc, router_ip, self.router_port, &request)?;
        match response
            .result_code_opt
            .expect("transact allowed absent result code")
        {
            ResultCode::Success => (),
            rc => return Err(AutomapError::TransactionFailure(format!("{:?}", rc))),
        }
        let opcode_data = response
            .opcode_data
            .as_any()
            .downcast_ref::<GetOpcodeData>()
            .expect("Response parsing inoperative - opcode data");
        let ip = opcode_data
            .external_ip_address_opt
            .expect("Response parsing inoperative - external IP address");
        Ok(IpAddr::V4(ip))
    }

    fn add_mapping(
        &self,
        router_ip: IpAddr,
        hole_port: u16,
        lifetime: u32,
    ) -> Result<u32, AutomapError> {
        self.mapping_adder_arc
            .lock()
            .expect("Housekeeping thread is dead")
            .add_mapping(&self.factories_arc,
            SocketAddr::new (router_ip, self.router_port), hole_port, lifetime)
            .map (|remap_interval| {
                self.change_handler_config_opt.replace (Some (ChangeHandlerConfig{ hole_port, lifetime }));
                remap_interval
            })
    }

    fn add_permanent_mapping(
        &self,
        _router_ip: IpAddr,
        _hole_port: u16,
    ) -> Result<u32, AutomapError> {
        panic!("PMP cannot add permanent mappings")
    }

    fn delete_mapping(&self, router_ip: IpAddr, hole_port: u16) -> Result<(), AutomapError> {
        self.add_mapping(router_ip, hole_port, 0)?;
        Ok(())
    }

    fn protocol(&self) -> AutomapProtocol {
        AutomapProtocol::Pmp
    }

    fn start_housekeeping_thread(
        &mut self,
        change_handler: ChangeHandler,
        router_ip: IpAddr,
    ) -> Result<Sender<HousekeepingThreadCommand>, AutomapError> {
        if let Some(_housekeeper_commander) = &self.housekeeper_commander_opt {
            return Err(AutomapError::ChangeHandlerAlreadyRunning);
        }
        let change_handler_config = match self.change_handler_config_opt.borrow().deref() {
            None => return Err(AutomapError::ChangeHandlerUnconfigured),
            Some(chc) => chc.clone(),
        };
        let ip_addr = IpAddr::V4(Ipv4Addr::new(224, 0, 0, 1));
        let socket_addr = SocketAddr::new(ip_addr, self.listen_port);
        let socket_result = {
            let factories = self.factories_arc.lock().expect("Automap is poisoned!");
            factories.socket_factory.make(socket_addr)
        };
        let socket = match socket_result {
            Ok(s) => s,
            Err(e) => {
                return Err(AutomapError::SocketBindingError(
                    format!("{:?}", e),
                    socket_addr,
                ))
            }
        };
        let (tx, rx) = unbounded();
        self.housekeeper_commander_opt = Some(tx.clone());
        let mapping_adder_arc = self.mapping_adder_arc.clone();
        let factories_arc = self.factories_arc.clone();
        let router_port = self.router_port;
        let read_timeout_millis = self.read_timeout_millis;
        let logger = self.logger.clone();
        thread::spawn(move || {
            Self::thread_guts(
                socket.as_ref(),
                &rx,
                mapping_adder_arc,
                factories_arc,
                router_ip,
                router_port,
                &change_handler,
                change_handler_config,
                read_timeout_millis,
                logger,
            )
        });
        Ok(tx)
    }

    fn stop_housekeeping_thread(&mut self) {
        if let Some(commander) = self.housekeeper_commander_opt.take() {
            let _ = commander.send(HousekeepingThreadCommand::Stop);
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl Default for PmpTransactor {
    fn default() -> Self {
        Self {
            mapping_adder_arc: Arc::new (Mutex::new (Box::new (MappingAdderReal {}))),
            factories_arc: Arc::new(Mutex::new(Factories::default())),
            router_port: ROUTER_PORT,
            listen_port: CHANGE_HANDLER_PORT,
            change_handler_config_opt: RefCell::new(None),
            housekeeper_commander_opt: None,
            read_timeout_millis: READ_TIMEOUT_MILLIS,
            logger: Logger::new("Automap"),
        }
    }
}

impl PmpTransactor {
    pub fn new() -> Self {
        Self::default()
    }

    fn transact(
        factories_arc: &Arc<Mutex<Factories>>,
        router_ip: IpAddr,
        router_port: u16,
        request: &PmpPacket,
    ) -> Result<PmpPacket, AutomapError> {
        let mut buffer = [0u8; 1100];
        let len = request
            .marshal(&mut buffer)
            .expect("Bad packet construction");
        let socket = {
            let factories = factories_arc.lock().expect("Factories are dead");
            let local_address =
                make_local_socket_address(router_ip, factories.free_port_factory.make());
            match factories.socket_factory.make(local_address) {
                Ok(s) => s,
                Err(e) => {
                    return Err(AutomapError::SocketBindingError(
                        format!("{:?}", e),
                        local_address,
                    ))
                }
            }
        };
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .expect("set_read_timeout failed");
        if let Err(e) = socket.send_to(&buffer[0..len], SocketAddr::new(router_ip, router_port)) {
            return Err(AutomapError::SocketSendError(AutomapErrorCause::Unknown(
                format!("{:?}", e),
            )));
        }
        let (len, _) = match socket.recv_from(&mut buffer) {
            Ok(len) => len,
            Err(e) if (e.kind() == ErrorKind::WouldBlock) || (e.kind() == ErrorKind::TimedOut) => {
                return Err(AutomapError::ProtocolError(
                    "Timed out after 3 seconds".to_string(),
                ))
            }
            Err(e) => {
                return Err(AutomapError::SocketReceiveError(
                    AutomapErrorCause::Unknown(format!("{:?}", e)),
                ))
            }
        };
        let response = match PmpPacket::try_from(&buffer[0..len]) {
            Ok(pkt) => pkt,
            Err(e) => return Err(AutomapError::PacketParseError(e)),
        };
        Ok(response)
    }

    fn thread_guts(
        announcement_socket: &dyn UdpSocketWrapper,
        rx: &Receiver<HousekeepingThreadCommand>,
        mapping_adder_arc: Arc<Mutex<Box<dyn MappingAdder>>>,
        factories_arc: Arc<Mutex<Factories>>,
        router_ip: IpAddr, // TODO: Make this a single SocketAddr
        router_port: u16,
        change_handler: &ChangeHandler,
        change_handler_config: ChangeHandlerConfig,
        read_timeout_millis: u64,
        logger: Logger,
    ) {
        let mut last_remapped = Instant::now();
        let mut remap_interval = Duration::from_secs(change_handler_config.lifetime as u64);
        announcement_socket
            .set_read_timeout(Some(Duration::from_millis(read_timeout_millis)))
            .expect("Can't set read timeout");
        loop {
            let mut buffer = [0u8; 100];
            match announcement_socket.recv_from(&mut buffer) {
                Ok((_, announcement_source_address)) => {
                    if announcement_source_address.ip() != router_ip {
                        continue;
                    }
                    match Self::parse_buffer(&buffer, announcement_source_address, &logger) {
                        Ok(public_ip) => {
                            let router_address = SocketAddr::new(router_ip, router_port);
                            Self::handle_announcement(
                                factories_arc.clone(),
                                router_address,
                                public_ip,
                                change_handler,
                                &change_handler_config,
                                &logger,
                            );
                        }
                        Err(_) => continue, // log already generated by parse_buffer()
                    }
                }
                Err(e) if (e.kind() == ErrorKind::WouldBlock) || (e.kind() == ErrorKind::TimedOut) => {
                    ()
                }
                Err(e) => error!(logger, "Error receiving PCP packet from router: {:?}", e),
            }
            let since_last_remapped = last_remapped.elapsed();
            if since_last_remapped.gt (&remap_interval) {
                let mapping_adder = mapping_adder_arc
                    .lock()
                    .expect("PcpTransactor is dead");
                if let Err (_e) = Self::remap_port(
                    (*mapping_adder).as_ref(),
                    &factories_arc,
                    SocketAddr::new (router_ip, router_port),
                    change_handler_config.hole_port,
                    Duration::from_secs (change_handler_config.lifetime as u64),
                    &logger,
                ) {
                    todo! ();
                    // error! (logger, "Remapping failure: {:?}", e);
                    // change_handler (AutomapChange::Error(e));
                }
                last_remapped = Instant::now();
            }
            match rx.try_recv () {
                Ok(HousekeepingThreadCommand::Stop) => break,
                Ok(HousekeepingThreadCommand::SetRemapIntervalMs(remap_after)) =>
                    remap_interval = Duration::from_millis(remap_after),
                Err (_) => (),
            }
        }
    }

    fn remap_port (
        mapping_adder: &dyn MappingAdder,
        factories_arc: &Arc<Mutex<Factories>>,
        router_addr: SocketAddr,
        hole_port: u16,
        requested_lifetime: Duration,
        logger: &Logger,
    ) -> Result<u32, AutomapError> {
        info! (logger, "Remapping port {}", hole_port);
        let mut requested_lifetime_secs = requested_lifetime.as_secs() as u32;
        if requested_lifetime_secs < 1 {
            requested_lifetime_secs = 1;
        }
        // TODO: Change the ChangeHandlerConfig's lifetime if this succeeds
        Ok(mapping_adder.add_mapping(factories_arc, router_addr, hole_port, requested_lifetime_secs)?)
    }

    fn parse_buffer(
        buffer: &[u8],
        source_address: SocketAddr,
        logger: &Logger,
    ) -> Result<Ipv4Addr, AutomapError> {
        match PmpPacket::try_from(buffer) {
            Ok(packet) => {
                if packet.direction != Direction::Response {
                    let err_msg = format!(
                        "Unexpected PMP Get request (request!) from router at {}: ignoring",
                        source_address
                    );
                    warning!(logger, "{}", err_msg);
                    return Err(AutomapError::ProtocolError(err_msg));
                }
                if packet.opcode == Opcode::Get {
                    let opcode_data = packet
                        .opcode_data
                        .as_any()
                        .downcast_ref::<GetOpcodeData>()
                        .expect("A Get opcode shouldn't parse anything but GetOpcodeData");
                    Ok(opcode_data
                        .external_ip_address_opt
                        .expect("A Response should always produce an external ip address"))
                } else {
                    let err_msg = format!(
                        "Unexpected PMP {:?} response (instead of Get) from router at {}: ignoring",
                        packet.opcode, source_address
                    );
                    warning!(logger, "{}", err_msg);
                    return Err(AutomapError::ProtocolError(err_msg));
                }
            }
            Err(_) => {
                error!(
                    logger,
                    "Unparseable PMP packet:\n{}",
                    PrettyHex::hex_dump(&buffer)
                );
                let err_msg = format!(
                    "Unparseable packet from router at {}: ignoring",
                    source_address
                );
                warning!(logger, "{}\n{}", err_msg, PrettyHex::hex_dump(&buffer));
                return Err(AutomapError::ProtocolError(err_msg));
            }
        }
    }

    fn handle_announcement(
        factories_arc: Arc<Mutex<Factories>>,
        router_address: SocketAddr,
        public_ip: Ipv4Addr,
        change_handler: &ChangeHandler,
        change_handler_config: &ChangeHandlerConfig,
        logger: &Logger,
    ) {
        let mut packet = PmpPacket {
            opcode: Opcode::MapTcp,
            direction: Direction::Request,
            ..Default::default()
        };
        let opcode_data = MapOpcodeData {
            epoch_opt: None,
            internal_port: change_handler_config.hole_port,
            external_port: change_handler_config.hole_port,
            lifetime: change_handler_config.lifetime,
        };
        packet.opcode_data = Box::new(opcode_data);
        debug!(
            logger,
            "Sending mapping request to {} and waiting for response", router_address
        );
        match Self::transact(
            &factories_arc,
            router_address.ip(),
            router_address.port(),
            &packet,
        ) {
            Ok(response) => match response.result_code_opt {
                Some(ResultCode::Success) => {
                    debug!(logger, "Prod: Received response; triggering change handler");
                    change_handler(AutomapChange::NewIp(IpAddr::V4(public_ip)));
                }
                Some(result_code) => {
                    let err_msg = format!(
                        "Remapping after IP change failed; Node is useless: {:?}",
                        result_code
                    );
                    error!(logger, "{}\n{:?}", err_msg, packet);
                    let automap_error = if result_code.is_permanent() {
                        AutomapError::PermanentMappingError(err_msg)
                    }
                    else {
                        AutomapError::TemporaryMappingError(err_msg)
                    };
                    change_handler(AutomapChange::Error(automap_error));
                    return;
                }
                None => {
                    let err_msg = "Remapping after IP change failed; Node is useless: Received request when expecting response".to_string();
                    error!(logger, "{}\n{:?}", err_msg, packet);
                    change_handler(AutomapChange::Error(AutomapError::ProtocolError(err_msg)));
                    return;
                }
            },
            Err(e) => {
                error!(
                    logger,
                    "Remapping after IP change failed; Node is useless: {:?}", e
                );
                change_handler(AutomapChange::Error(AutomapError::SocketReceiveError(
                    AutomapErrorCause::SocketFailure,
                )));
            }
        }
    }
}

trait MappingAdder: Send {
    // TODO: Maybe substitute ChangeHandlerConfig for hole_port and lifetime
    fn add_mapping(&self, factories_arc: &Arc<Mutex<Factories>>, router_addr: SocketAddr,
                   hole_port: u16, lifetime: u32) -> Result<u32, AutomapError>;
}

#[derive (Clone)]
struct MappingAdderReal {}

impl Default for MappingAdderReal {
    fn default() -> Self {
        Self{}
    }
}

impl MappingAdder for MappingAdderReal {
    fn add_mapping(&self, factories_arc: &Arc<Mutex<Factories>>, router_addr: SocketAddr,
                   hole_port: u16, lifetime: u32) -> Result<u32, AutomapError> {
        let request = PmpPacket {
            direction: Direction::Request,
            opcode: Opcode::MapTcp,
            result_code_opt: None,
            opcode_data: Box::new(MapOpcodeData {
                epoch_opt: None,
                internal_port: hole_port,
                external_port: hole_port,
                lifetime,
            }),
        };
        let response = PmpTransactor::transact(factories_arc, router_addr.ip(),
                                               router_addr.port(), &request)?;
        if response.direction == Direction::Request {
            return Err (AutomapError::ProtocolError ("Map response labeled as request".to_string()))
        }
        if response.opcode != Opcode::MapTcp {
            return Err(AutomapError::ProtocolError(format! ("Expected MapTcp response; got {:?} response instead",
                                                            response.opcode)));
        }
        let opcode_data: &MapOpcodeData = response.opcode_data.as_any().downcast_ref()
            .expect ("MapTcp response contained other than MapOpcodeData");
        match response
            .result_code_opt
            .expect("transact allowed absent result code")
        {
            ResultCode::Success => Ok(opcode_data.lifetime / 2),
            rc => {
                let msg = format!("{:?}", rc);
                Err (if rc.is_permanent() {
                    AutomapError::PermanentMappingError(msg)
                }
                else {
                    AutomapError::TemporaryMappingError(msg)
                })
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::comm_layer::pcp_pmp_common::mocks::{
        FreePortFactoryMock, UdpSocketFactoryMock, UdpSocketMock,
    };
    use crate::comm_layer::pcp_pmp_common::{ChangeHandlerConfig, UdpSocket};
    use crate::comm_layer::AutomapErrorCause;
    use crate::control_layer::automap_control::AutomapChange;
    use crate::protocols::pmp::get_packet::GetOpcodeData;
    use crate::protocols::pmp::map_packet::MapOpcodeData;
    use crate::protocols::pmp::pmp_packet::{Opcode, PmpOpcodeData, PmpPacket, ResultCode};
    use crate::protocols::utils::{Direction, Packet, ParseError, UnrecognizedData};
    use masq_lib::test_utils::logging::{init_test_logging, TestLogHandler};
    use masq_lib::utils::{find_free_port, localhost, AutomapProtocol};
    use std::cell::RefCell;
    use std::io::ErrorKind;
    use std::net::{Ipv4Addr, SocketAddr};
    use std::str::FromStr;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use std::{io, thread};

    struct MappingAdderMock {
        add_mapping_params: Arc<Mutex<Vec<(Arc<Mutex<Factories>>, SocketAddr, u16, u32)>>>,
        add_mapping_results: RefCell<Vec<Result<u32, AutomapError>>>,
    }

    impl MappingAdder for MappingAdderMock {
        fn add_mapping(&self, factories_arc: &Arc<Mutex<Factories>>, router_addr: SocketAddr,
                       hole_port: u16, lifetime: u32) -> Result<u32, AutomapError> {
            self.add_mapping_params.lock().unwrap().push ((factories_arc.clone(), router_addr,
                hole_port, lifetime));
            self.add_mapping_results.borrow_mut().remove (0)
        }
    }

    impl MappingAdderMock {
        fn new () -> Self {
            Self {
                add_mapping_params: Default::default(),
                add_mapping_results: Default::default(),
            }
        }

        fn add_mapping_params (mut self, params: &Arc<Mutex<Vec<(Arc<Mutex<Factories>>, SocketAddr, u16, u32)>>>) -> Self {
            self.add_mapping_params = params.clone();
            self
        }

        fn add_mapping_result (self, result: Result<u32, AutomapError>) -> Self {
            self.add_mapping_results.borrow_mut().push (result);
            self
        }
    }

    #[test]
    fn knows_its_method() {
        let subject = PmpTransactor::new();

        let method = subject.protocol();

        assert_eq!(method, AutomapProtocol::Pmp);
    }

    #[test]
    fn transact_handles_socket_binding_error() {
        let router_ip = IpAddr::from_str("1.2.3.4").unwrap();
        let io_error = io::Error::from(ErrorKind::ConnectionReset);
        let io_error_str = format!("{:?}", io_error);
        let socket_factory = UdpSocketFactoryMock::new().make_result(Err(io_error));
        let subject = make_subject(socket_factory);

        let result = subject.get_public_ip(router_ip).err().unwrap();

        match result {
            AutomapError::SocketBindingError(msg, addr) => {
                assert_eq!(msg, io_error_str);
                assert_eq!(addr.ip(), IpAddr::from_str("0.0.0.0").unwrap());
                assert_eq!(addr.port(), 5566);
            }
            e => panic!("Expected SocketBindingError, got {:?}", e),
        }
    }

    #[test]
    fn transact_handles_socket_send_error() {
        let router_ip = IpAddr::from_str("1.2.3.4").unwrap();
        let io_error = io::Error::from(ErrorKind::ConnectionReset);
        let io_error_str = format!("{:?}", io_error);
        let socket = UdpSocketMock::new()
            .set_read_timeout_result(Ok(()))
            .send_to_result(Err(io_error));
        let socket_factory = UdpSocketFactoryMock::new().make_result(Ok(socket));
        let subject = make_subject(socket_factory);

        let result = subject.add_mapping(router_ip, 7777, 1234);

        assert_eq!(
            result,
            Err(AutomapError::SocketSendError(AutomapErrorCause::Unknown(
                io_error_str
            )))
        );
    }

    #[test]
    fn transact_handles_socket_receive_error() {
        let router_ip = IpAddr::from_str("1.2.3.4").unwrap();
        let io_error = io::Error::from(ErrorKind::ConnectionReset);
        let io_error_str = format!("{:?}", io_error);
        let socket = UdpSocketMock::new()
            .set_read_timeout_result(Ok(()))
            .send_to_result(Ok(24))
            .recv_from_result(Err(io_error), vec![]);
        let socket_factory = UdpSocketFactoryMock::new().make_result(Ok(socket));
        let subject = make_subject(socket_factory);

        let result = subject.add_mapping(router_ip, 7777, 1234);

        assert_eq!(
            result,
            Err(AutomapError::SocketReceiveError(
                AutomapErrorCause::Unknown(io_error_str)
            ))
        );
    }

    #[test]
    fn transact_handles_packet_parse_error() {
        let router_ip = IpAddr::from_str("1.2.3.4").unwrap();
        let socket = UdpSocketMock::new()
            .set_read_timeout_result(Ok(()))
            .send_to_result(Ok(24))
            .recv_from_result(Ok((0, SocketAddr::new(router_ip, ROUTER_PORT))), vec![]);
        let socket_factory = UdpSocketFactoryMock::new().make_result(Ok(socket));
        let subject = PmpTransactor::default();
        subject.factories_arc.lock().unwrap().socket_factory = Box::new(socket_factory);

        let result = subject.add_mapping(router_ip, 7777, 1234);

        assert_eq!(
            result,
            Err(AutomapError::PacketParseError(ParseError::ShortBuffer(
                2, 0
            )))
        );
    }

    #[test]
    fn find_routers_returns_something_believable() {
        let subject = PmpTransactor::default();

        let result = subject.find_routers().unwrap();

        assert_eq!(result.len(), 1);
    }

    #[test]
    fn add_mapping_handles_socket_factory_error() {
        let router_ip = IpAddr::from_str("192.168.0.1").unwrap();
        let io_error = io::Error::from(ErrorKind::ConnectionRefused);
        let io_error_str = format!("{:?}", io_error);
        let socket_factory = UdpSocketFactoryMock::new().make_result(Err(io_error));
        let free_port_factory = FreePortFactoryMock::new().make_result(5566);
        let subject = MappingAdderReal::default();
        let mut factories = Factories::default();
        factories.socket_factory = Box::new(socket_factory);
        factories.free_port_factory = Box::new (free_port_factory);

        let result = subject.add_mapping(
            &Arc::new (Mutex::new (factories)),
            SocketAddr::new (router_ip, ROUTER_PORT),
            6666,
            4321,
        )
            .err()
            .unwrap();

        match result {
            AutomapError::SocketBindingError(msg, addr) => {
                assert_eq!(msg, io_error_str);
                assert_eq!(addr.ip(), IpAddr::from_str("0.0.0.0").unwrap());
                assert_eq!(addr.port(), 5566);
            }
            e => panic!("Expected SocketBindingError, got {:?}", e),
        }
    }

    #[test]
    fn add_mapping_handles_send_to_error() {
        let router_ip = IpAddr::from_str("192.168.0.1").unwrap();
        let io_error = io::Error::from(ErrorKind::ConnectionRefused);
        let io_error_str = format!("{:?}", io_error);
        let socket = UdpSocketMock::new()
            .set_read_timeout_result(Ok(()))
            .send_to_result(Err(io_error));
        let socket_factory = UdpSocketFactoryMock::new().make_result(Ok(socket));
        let subject = MappingAdderReal::default();
        let mut factories = Factories::default();
        factories.socket_factory = Box::new(socket_factory);

        let result = subject.add_mapping(
            &Arc::new (Mutex::new (factories)),
            SocketAddr::new (router_ip, ROUTER_PORT),
            6666,
            4321,
        );

        assert_eq!(
            result,
            Err(AutomapError::SocketSendError(AutomapErrorCause::Unknown(
                io_error_str
            )))
        );
    }

    #[test]
    fn add_mapping_handles_recv_from_error() {
        let router_ip = IpAddr::from_str("192.168.0.1").unwrap();
        let io_error = io::Error::from(ErrorKind::ConnectionRefused);
        let io_error_str = format!("{:?}", io_error);
        let socket = UdpSocketMock::new()
            .set_read_timeout_result(Ok(()))
            .send_to_result(Ok(1000))
            .recv_from_result(Err(io_error), vec![]);
        let socket_factory = UdpSocketFactoryMock::new().make_result(Ok(socket));
        let subject = MappingAdderReal::default();
        let mut factories = Factories::default();
        factories.socket_factory = Box::new(socket_factory);

        let result = subject.add_mapping(
            &Arc::new (Mutex::new (factories)),
            SocketAddr::new (router_ip, ROUTER_PORT),
            6666,
            4321,
        );

        assert_eq!(
            result,
            Err(AutomapError::SocketReceiveError(
                AutomapErrorCause::Unknown(io_error_str)
            ))
        );
    }

    #[test]
    fn add_mapping_handles_packet_parse_error() {
        let router_ip = IpAddr::from_str("192.168.0.1").unwrap();
        let socket = UdpSocketMock::new()
            .set_read_timeout_result(Ok(()))
            .send_to_result(Ok(1000))
            .recv_from_result(Ok((0, SocketAddr::new(router_ip, ROUTER_PORT))), vec![]);
        let socket_factory = UdpSocketFactoryMock::new().make_result(Ok(socket));
        let subject = MappingAdderReal::default();
        let mut factories = Factories::default();
        factories.socket_factory = Box::new(socket_factory);

        let result = subject.add_mapping(
            &Arc::new (Mutex::new (factories)),
            SocketAddr::new (router_ip, ROUTER_PORT),
            6666,
            4321,
        );

        assert_eq!(
            result,
            Err(AutomapError::PacketParseError(ParseError::ShortBuffer(
                2, 0
            )))
        );
    }

    #[test]
    fn add_mapping_handles_wrong_direction() {
        let router_ip = IpAddr::from_str("192.168.0.1").unwrap();
        let mut buffer = [0u8; 1100];
        let packet = make_request(Opcode::Other(127), Box::new(UnrecognizedData::new()));
        let len = packet.marshal(&mut buffer).unwrap();
        let socket = UdpSocketMock::new()
            .set_read_timeout_result(Ok(()))
            .send_to_result(Ok(1000))
            .recv_from_result(
                Ok((len, SocketAddr::new(router_ip, ROUTER_PORT))),
                buffer[0..len].to_vec(),
            );
        let socket_factory = UdpSocketFactoryMock::new().make_result(Ok(socket));
        let subject = MappingAdderReal::default();
        let mut factories = Factories::default();
        factories.socket_factory = Box::new(socket_factory);

        let result = subject.add_mapping(
            &Arc::new (Mutex::new (factories)),
            SocketAddr::new (router_ip, ROUTER_PORT),
            6666,
            4321,
        );

        assert_eq!(
            result,
            Err(AutomapError::ProtocolError(
                "Map response labeled as request".to_string()
            ))
        );
    }

    #[test]
    fn add_mapping_handles_unexpected_opcode() {
        let router_ip = IpAddr::from_str("192.168.0.1").unwrap();
        let mut buffer = [0u8; 1100];
        let mut packet = make_response(Opcode::Other(127),
            ResultCode::Success, Box::new(UnrecognizedData::new()));
        packet.opcode = Opcode::Other(127);
        let len = packet.marshal(&mut buffer).unwrap();
        let socket = UdpSocketMock::new()
            .set_read_timeout_result(Ok(()))
            .send_to_result(Ok(1000))
            .recv_from_result(
                Ok((len, SocketAddr::new(router_ip, ROUTER_PORT))),
                buffer[0..len].to_vec(),
            );
        let socket_factory = UdpSocketFactoryMock::new().make_result(Ok(socket));
        let subject = MappingAdderReal::default();
        let mut factories = Factories::default();
        factories.socket_factory = Box::new(socket_factory);

        let result = subject.add_mapping(
            &Arc::new (Mutex::new (factories)),
            SocketAddr::new (router_ip, ROUTER_PORT),
            6666,
            4321,
        );

        assert_eq!(
            result,
            Err(AutomapError::ProtocolError(
                "Expected MapTcp response; got Other(127) response instead".to_string()
            ))
        );
    }

    #[test]
    fn get_public_ip_works() {
        let router_ip = IpAddr::from_str("1.2.3.4").unwrap();
        let public_ip = Ipv4Addr::from_str("72.73.74.75").unwrap();
        let mut request_buffer = [0u8; 1100];
        let request = make_request(Opcode::Get, make_get_request());
        let request_len = request.marshal(&mut request_buffer).unwrap();
        let mut response_buffer = [0u8; 1100];
        let response = make_response(
            Opcode::Get,
            ResultCode::Success,
            make_get_response(1234, public_ip),
        );
        let response_len = response.marshal(&mut response_buffer).unwrap();
        let set_read_timeout_params_arc = Arc::new(Mutex::new(vec![]));
        let send_to_params_arc = Arc::new(Mutex::new(vec![]));
        let recv_from_params_arc = Arc::new(Mutex::new(vec![]));
        let socket = UdpSocketMock::new()
            .set_read_timeout_params(&set_read_timeout_params_arc)
            .set_read_timeout_result(Ok(()))
            .send_to_params(&send_to_params_arc)
            .send_to_result(Ok(request_len))
            .recv_from_params(&recv_from_params_arc)
            .recv_from_result(
                Ok((response_len, SocketAddr::new(router_ip, ROUTER_PORT))),
                response_buffer[0..response_len].to_vec(),
            );
        let socket_factory = UdpSocketFactoryMock::new().make_result(Ok(socket));
        let subject = make_subject(socket_factory);

        let result = subject.get_public_ip(router_ip);

        assert_eq!(result, Ok(IpAddr::V4(public_ip)));
        let set_read_timeout_params = set_read_timeout_params_arc.lock().unwrap();
        assert_eq!(
            *set_read_timeout_params,
            vec![Some(Duration::from_millis(3000))]
        );
        let send_to_params = send_to_params_arc.lock().unwrap();
        assert_eq!(
            *send_to_params,
            vec![(
                request_buffer[0..request_len].to_vec(),
                SocketAddr::new(router_ip, ROUTER_PORT)
            )]
        );
        let recv_from_params = recv_from_params_arc.lock().unwrap();
        assert_eq!(*recv_from_params, vec![()])
    }

    #[test]
    fn get_public_ip_handles_unsuccessful_result_code() {
        let router_ip = IpAddr::from_str("1.2.3.4").unwrap();
        let public_ip = Ipv4Addr::from_str("72.73.74.75").unwrap();
        let mut response_buffer = [0u8; 1100];
        let mut response = make_response(
            Opcode::Get,
            ResultCode::Success,
            make_get_response(1234, public_ip),
        );
        response.result_code_opt = Some(ResultCode::OutOfResources);
        let response_len = response.marshal(&mut response_buffer).unwrap();
        let socket = UdpSocketMock::new()
            .set_read_timeout_result(Ok(()))
            .send_to_result(Ok(24))
            .recv_from_result(
                Ok((response_len, SocketAddr::new(router_ip, ROUTER_PORT))),
                response_buffer[0..response_len].to_vec(),
            );
        let socket_factory = UdpSocketFactoryMock::new().make_result(Ok(socket));
        let subject = make_subject(socket_factory);

        let result = subject.get_public_ip(router_ip);

        assert_eq!(
            result,
            Err(AutomapError::TransactionFailure(
                "OutOfResources".to_string()
            ))
        );
    }

    #[test]
    fn add_mapping_works() {
        let router_ip = IpAddr::from_str("1.2.3.4").unwrap();
        let mut request_buffer = [0u8; 1100];
        let request = make_request(Opcode::MapTcp, make_map_request(7777, 10000));
        let request_len = request.marshal(&mut request_buffer).unwrap();
        let mut response_buffer = [0u8; 1100];
        let response = make_response(
            Opcode::MapTcp,
            ResultCode::Success,
            make_map_response(4321, 7777, 8000),
        );
        let response_len = response.marshal(&mut response_buffer).unwrap();
        let set_read_timeout_params_arc = Arc::new(Mutex::new(vec![]));
        let send_to_params_arc = Arc::new(Mutex::new(vec![]));
        let recv_from_params_arc = Arc::new(Mutex::new(vec![]));
        let socket = UdpSocketMock::new()
            .set_read_timeout_params(&set_read_timeout_params_arc)
            .set_read_timeout_result(Ok(()))
            .send_to_params(&send_to_params_arc)
            .send_to_result(Ok(request_len))
            .recv_from_params(&recv_from_params_arc)
            .recv_from_result(
                Ok((response_len, SocketAddr::new(router_ip, ROUTER_PORT))),
                response_buffer[0..response_len].to_vec(),
            );
        let socket_factory = UdpSocketFactoryMock::new().make_result(Ok(socket));
        let subject = make_subject(socket_factory);

        let result = subject.add_mapping(router_ip, 7777, 10000);

        assert_eq!(result, Ok(4000));
        assert_eq!(subject.change_handler_config_opt.borrow().as_ref(), Some (&ChangeHandlerConfig{ hole_port: 7777, lifetime: 10000 }));
        let set_read_timeout_params = set_read_timeout_params_arc.lock().unwrap();
        assert_eq!(
            *set_read_timeout_params,
            vec![Some(Duration::from_millis(3000))]
        );
        let send_to_params = send_to_params_arc.lock().unwrap();
        assert_eq!(
            *send_to_params,
            vec![(
                request_buffer[0..request_len].to_vec(),
                SocketAddr::new(router_ip, ROUTER_PORT)
            )]
        );
        let recv_from_params = recv_from_params_arc.lock().unwrap();
        assert_eq!(*recv_from_params, vec![()])
    }

    #[test]
    fn add_mapping_handles_temporarily_unsuccessful_result_code() {
        let router_ip = IpAddr::from_str("1.2.3.4").unwrap();
        let mut response_buffer = [0u8; 1100];
        let mut response = make_response(
            Opcode::MapTcp,
            ResultCode::Success,
            make_map_response(4321, 7777, 1234),
        );
        response.result_code_opt = Some(ResultCode::OutOfResources);
        let response_len = response.marshal(&mut response_buffer).unwrap();
        let socket = UdpSocketMock::new()
            .set_read_timeout_result(Ok(()))
            .send_to_result(Ok(24))
            .recv_from_result(
                Ok((response_len, SocketAddr::new(router_ip, ROUTER_PORT))),
                response_buffer[0..response_len].to_vec(),
            );
        let socket_factory = UdpSocketFactoryMock::new().make_result(Ok(socket));
        let subject = make_subject(socket_factory);

        let result = subject.add_mapping(router_ip, 7777, 1234);

        assert_eq!(
            result,
            Err(AutomapError::TemporaryMappingError(
                "OutOfResources".to_string()
            ))
        );
    }

    #[test]
    fn add_mapping_handles_permanently_unsuccessful_result_code() {
        let router_ip = IpAddr::from_str("1.2.3.4").unwrap();
        let mut response_buffer = [0u8; 1100];
        let mut response = make_response(
            Opcode::MapTcp,
            ResultCode::Success,
            make_map_response(4321, 7777, 1234),
        );
        response.result_code_opt = Some(ResultCode::UnsupportedOpcode);
        let response_len = response.marshal(&mut response_buffer).unwrap();
        let socket = UdpSocketMock::new()
            .set_read_timeout_result(Ok(()))
            .send_to_result(Ok(24))
            .recv_from_result(
                Ok((response_len, SocketAddr::new(router_ip, ROUTER_PORT))),
                response_buffer[0..response_len].to_vec(),
            );
        let socket_factory = UdpSocketFactoryMock::new().make_result(Ok(socket));
        let subject = make_subject(socket_factory);

        let result = subject.add_mapping(router_ip, 7777, 1234);

        assert_eq!(
            result,
            Err(AutomapError::PermanentMappingError(
                "UnsupportedOpcode".to_string()
            ))
        );
    }

    #[test]
    #[should_panic(expected = "PMP cannot add permanent mappings")]
    fn add_permanent_mapping_is_not_implemented() {
        let subject = PmpTransactor::default();

        let _ = subject.add_permanent_mapping(IpAddr::from_str("0.0.0.0").unwrap(), 0);
    }

    #[test]
    fn delete_mapping_works() {
        let router_ip = IpAddr::from_str("1.2.3.4").unwrap();
        let mut request_buffer = [0u8; 1100];
        let request = PmpPacket {
            direction: Direction::Request,
            opcode: Opcode::MapTcp,
            result_code_opt: None,
            opcode_data: Box::new(MapOpcodeData {
                epoch_opt: None,
                internal_port: 7777,
                external_port: 7777,
                lifetime: 0,
            }),
        };
        let request_len = request.marshal(&mut request_buffer).unwrap();
        let mut response_buffer = [0u8; 1100];
        let response = PmpPacket {
            direction: Direction::Response,
            opcode: Opcode::MapTcp,
            result_code_opt: Some(ResultCode::Success),
            opcode_data: Box::new(MapOpcodeData {
                epoch_opt: Some(4321),
                internal_port: 7777,
                external_port: 7777,
                lifetime: 0,
            }),
        };
        let response_len = response.marshal(&mut response_buffer).unwrap();
        let set_read_timeout_params_arc = Arc::new(Mutex::new(vec![]));
        let send_to_params_arc = Arc::new(Mutex::new(vec![]));
        let recv_from_params_arc = Arc::new(Mutex::new(vec![]));
        let socket = UdpSocketMock::new()
            .set_read_timeout_params(&set_read_timeout_params_arc)
            .set_read_timeout_result(Ok(()))
            .send_to_params(&send_to_params_arc)
            .send_to_result(Ok(request_len))
            .recv_from_params(&recv_from_params_arc)
            .recv_from_result(
                Ok((response_len, SocketAddr::new(router_ip, ROUTER_PORT))),
                response_buffer[0..response_len].to_vec(),
            );
        let socket_factory = UdpSocketFactoryMock::new().make_result(Ok(socket));
        let subject = make_subject(socket_factory);

        let result = subject.delete_mapping(router_ip, 7777);

        assert_eq!(result, Ok(()));
        let set_read_timeout_params = set_read_timeout_params_arc.lock().unwrap();
        assert_eq!(
            *set_read_timeout_params,
            vec![Some(Duration::from_millis(3000))]
        );
        let send_to_params = send_to_params_arc.lock().unwrap();
        assert_eq!(
            *send_to_params,
            vec![(
                request_buffer[0..request_len].to_vec(),
                SocketAddr::new(router_ip, ROUTER_PORT)
            )]
        );
        let recv_from_params = recv_from_params_arc.lock().unwrap();
        assert_eq!(*recv_from_params, vec![()])
    }

    #[test]
    fn change_handler_works() {
        let change_handler_port = find_free_port();
        let router_port = find_free_port();
        let mut subject = PmpTransactor::default();
        subject.router_port = router_port;
        subject.listen_port = change_handler_port;
        subject.change_handler_config_opt = RefCell::new(Some(ChangeHandlerConfig {
            hole_port: 1234,
            lifetime: 321,
        }));
        let changes_arc = Arc::new(Mutex::new(vec![]));
        let changes_arc_inner = changes_arc.clone();
        let change_handler = move |change| {
            changes_arc_inner.lock().unwrap().push(change);
        };

        subject
            .start_housekeeping_thread(Box::new(change_handler), localhost())
            .unwrap();

        assert!(subject.housekeeper_commander_opt.is_some());
        let change_handler_ip = IpAddr::from_str("224.0.0.1").unwrap();
        let announce_socket = UdpSocket::bind(SocketAddr::new(localhost(), 0)).unwrap();
        announce_socket
            .set_read_timeout(Some(Duration::from_millis(1000)))
            .unwrap();
        announce_socket.set_broadcast(true).unwrap();
        announce_socket
            .connect(SocketAddr::new(change_handler_ip, change_handler_port))
            .unwrap();
        let mut packet = PmpPacket::default();
        packet.opcode = Opcode::Get;
        packet.direction = Direction::Response;
        packet.result_code_opt = Some(ResultCode::Success);
        packet.opcode_data = make_get_response(0, Ipv4Addr::from_str("1.2.3.4").unwrap());
        let mut buffer = [0u8; 100];
        let len_to_send = packet.marshal(&mut buffer).unwrap();
        let mapping_target_address = SocketAddr::new(localhost(), router_port);
        let mapping_socket = UdpSocket::bind(mapping_target_address).unwrap();
        let sent_len = announce_socket.send(&buffer[0..len_to_send]).unwrap();
        assert_eq!(sent_len, len_to_send);
        mapping_socket
            .set_read_timeout(Some(Duration::from_millis(1000)))
            .unwrap();
        let (recv_len, remapping_socket_addr) = mapping_socket.recv_from(&mut buffer).unwrap();
        let packet = PmpPacket::try_from(&buffer[0..recv_len]).unwrap();
        assert_eq!(packet.opcode, Opcode::MapTcp);
        let opcode_data: &MapOpcodeData = packet.opcode_data.as_any().downcast_ref().unwrap();
        assert_eq!(opcode_data.external_port, 1234);
        assert_eq!(opcode_data.internal_port, 1234);
        let mut packet = PmpPacket::default();
        packet.opcode = Opcode::MapTcp;
        packet.direction = Direction::Response;
        packet.result_code_opt = Some(ResultCode::Success);
        packet.opcode_data = make_map_response(0, 1234, 0);
        let len_to_send = packet.marshal(&mut buffer).unwrap();
        let sent_len = mapping_socket
            .send_to(&buffer[0..len_to_send], remapping_socket_addr)
            .unwrap();
        assert_eq!(sent_len, len_to_send);
        thread::sleep(Duration::from_millis(1)); // yield timeslice
        subject.stop_housekeeping_thread();
        assert!(subject.housekeeper_commander_opt.is_none());
        let changes = changes_arc.lock().unwrap();
        assert_eq!(
            *changes,
            vec![AutomapChange::NewIp(IpAddr::from_str("1.2.3.4").unwrap())]
        );
    }

    #[test]
    fn change_handler_rejects_data_from_non_router_ip_addresses() {
        let change_handler_port = find_free_port();
        let router_port = find_free_port();
        let router_ip = IpAddr::from_str("7.7.7.7").unwrap();
        let mut subject = PmpTransactor::default();
        subject.router_port = router_port;
        subject.listen_port = change_handler_port;
        subject.change_handler_config_opt = RefCell::new(Some(ChangeHandlerConfig {
            hole_port: 1234,
            lifetime: 321,
        }));
        let changes_arc = Arc::new(Mutex::new(vec![]));
        let changes_arc_inner = changes_arc.clone();
        let change_handler = move |change| {
            changes_arc_inner.lock().unwrap().push(change);
        };

        subject
            .start_housekeeping_thread(Box::new(change_handler), router_ip)
            .unwrap();

        assert!(subject.housekeeper_commander_opt.is_some());
        let change_handler_ip = IpAddr::from_str("224.0.0.1").unwrap();
        let announce_socket = UdpSocket::bind(SocketAddr::new(localhost(), 0)).unwrap();
        announce_socket
            .set_read_timeout(Some(Duration::from_millis(1000)))
            .unwrap();
        announce_socket.set_broadcast(true).unwrap();
        announce_socket
            .connect(SocketAddr::new(change_handler_ip, change_handler_port))
            .unwrap();
        let mut packet = PmpPacket::default();
        packet.opcode = Opcode::Get;
        packet.direction = Direction::Response;
        packet.result_code_opt = Some(ResultCode::Success);
        packet.opcode_data = make_get_response(0, Ipv4Addr::from_str("1.2.3.4").unwrap());
        let mut buffer = [0u8; 100];
        let len_to_send = packet.marshal(&mut buffer).unwrap();
        let sent_len = announce_socket.send(&buffer[0..len_to_send]).unwrap();
        assert_eq!(sent_len, len_to_send);
        thread::sleep(Duration::from_millis(1)); // yield timeslice
        subject.stop_housekeeping_thread();
        assert!(subject.housekeeper_commander_opt.is_none());
        let changes = changes_arc.lock().unwrap();
        assert_eq!(*changes, vec![]);
    }

    #[test]
    fn change_handler_rejects_data_that_causes_parse_errors() {
        init_test_logging();
        let change_handler_port = find_free_port();
        let router_port = find_free_port();
        let router_ip = localhost();
        let mut subject = PmpTransactor::default();
        subject.router_port = router_port;
        subject.listen_port = change_handler_port;
        subject.change_handler_config_opt = RefCell::new(Some(ChangeHandlerConfig {
            hole_port: 1234,
            lifetime: 321,
        }));
        let changes_arc = Arc::new(Mutex::new(vec![]));
        let changes_arc_inner = changes_arc.clone();
        let change_handler = move |change| {
            changes_arc_inner.lock().unwrap().push(change);
        };

        subject
            .start_housekeeping_thread(Box::new(change_handler), router_ip)
            .unwrap();

        assert!(subject.housekeeper_commander_opt.is_some());
        let change_handler_ip = IpAddr::from_str("224.0.0.1").unwrap();
        let announce_socket = UdpSocket::bind(SocketAddr::new(localhost(), 0)).unwrap();
        announce_socket
            .set_read_timeout(Some(Duration::from_millis(1000)))
            .unwrap();
        announce_socket.set_broadcast(true).unwrap();
        announce_socket
            .connect(SocketAddr::new(change_handler_ip, change_handler_port))
            .unwrap();
        let mut packet = PmpPacket::default();
        packet.opcode = Opcode::Get;
        packet.direction = Direction::Request; // should be Response
        packet.result_code_opt = Some(ResultCode::Success);
        packet.opcode_data = make_get_response(0, Ipv4Addr::from_str("1.2.3.4").unwrap());
        let mut buffer = [0u8; 100];
        let len_to_send = packet.marshal(&mut buffer).unwrap();
        let sent_len = announce_socket.send(&buffer[0..len_to_send]).unwrap();
        assert_eq!(sent_len, len_to_send);
        thread::sleep(Duration::from_millis(1)); // yield timeslice
        subject.stop_housekeeping_thread();
        assert!(subject.housekeeper_commander_opt.is_none());
        let changes = changes_arc.lock().unwrap();
        assert_eq!(*changes, vec![]);
        let err_msg = "Unexpected PMP Get request (request!) from router at ";
        TestLogHandler::new().exists_log_containing(&format!("WARN: Automap: {}", err_msg));
    }

    #[test]
    fn thread_guts_does_not_remap_if_interval_does_not_run_out () {
        init_test_logging();
        let announcement_socket: Box<dyn UdpSocketWrapper> = Box::new(
            UdpSocketMock::new()
                .set_read_timeout_result(Ok(()))
                .recv_from_result(Err(io::Error::from(ErrorKind::TimedOut)), vec![])
        );
        let (tx, rx) = unbounded();
        let mapping_adder = Box::new (MappingAdderMock::new ()); // no results specified
        let change_handler: ChangeHandler = Box::new(move |_| {});
        let change_handler_config = ChangeHandlerConfig{ hole_port: 0, lifetime: 1000 };
        tx.send(HousekeepingThreadCommand::SetRemapIntervalMs(1000)).unwrap();
        tx.send(HousekeepingThreadCommand::Stop).unwrap();

        PmpTransactor::thread_guts(
            announcement_socket.as_ref(),
            &rx,
            Arc::new (Mutex::new (mapping_adder)),
            Arc::new(Mutex::new(Factories::default())),
            localhost(),
            0,
            &change_handler,
            change_handler_config,
            10,
            Logger::new ("no_remap_test")
        );

        TestLogHandler::new().exists_no_log_containing("INFO: no_remap_test: Remapping port 1234");
    }

    #[test]
    fn thread_guts_remaps_when_interval_runs_out () {
        init_test_logging();
        let (tx, rx) = unbounded();
        let add_mapping_params_arc = Arc::new (Mutex::new (vec![]));
        let mapping_adder = Box::new (MappingAdderMock::new ()
            .add_mapping_params(&add_mapping_params_arc)
            .add_mapping_result(Ok(300))
        );
        let free_port_factory = FreePortFactoryMock::new ().make_result (5555);
        let mut factories = Factories::default();
        factories.free_port_factory = Box::new (free_port_factory);
        let announcement_socket: Box<dyn UdpSocketWrapper> = Box::new(
            UdpSocketMock::new()
                .set_read_timeout_result(Ok(()))
                .recv_from_result(Err(io::Error::from(ErrorKind::WouldBlock)), vec![])
        );
        let change_handler: ChangeHandler = Box::new(move |_| {});
        let change_handler_config = ChangeHandlerConfig{ hole_port: 6689, lifetime: 1000 };
        tx.send(HousekeepingThreadCommand::SetRemapIntervalMs(80)).unwrap();

        let handle = thread::spawn (move || {
            PmpTransactor::thread_guts(
                announcement_socket.as_ref(),
                &rx,
                Arc::new (Mutex::new (mapping_adder)),
                Arc::new(Mutex::new(factories)),
                IpAddr::from_str ("6.6.6.6").unwrap(),
                6666,
                &change_handler,
                change_handler_config,
                10,
                Logger::new ("timed_remap_test")
            );
        });

        thread::sleep (Duration::from_millis(100));
        tx.send(HousekeepingThreadCommand::Stop).unwrap();
        handle.join().unwrap();
        let add_mapping_params = add_mapping_params_arc.lock().unwrap().remove(0);
        assert_eq! (add_mapping_params.0.lock().unwrap().free_port_factory.make (), 5555);
        assert_eq! (add_mapping_params.1, SocketAddr::from_str ("6.6.6.6:6666").unwrap());
        assert_eq! (add_mapping_params.2, 6689);
        assert_eq! (add_mapping_params.3, 1000);
        TestLogHandler::new().exists_log_containing("INFO: timed_remap_test: Remapping port 6689");
    }

    #[test]
    fn parse_buffer_rejects_request_packet() {
        init_test_logging();
        let router_ip = IpAddr::from_str("4.3.2.1").unwrap();
        let mut packet = PmpPacket::default();
        packet.opcode = Opcode::Get;
        packet.direction = Direction::Request;
        let mut buffer = [0u8; 100];
        let buflen = packet.marshal(&mut buffer).unwrap();
        let logger = Logger::new("PMPTransactor");

        let result = PmpTransactor::parse_buffer(
            &buffer[0..buflen],
            SocketAddr::new(router_ip, 5351),
            &logger,
        );

        let err_msg = "Unexpected PMP Get request (request!) from router at 4.3.2.1:5351: ignoring";
        assert_eq!(
            result,
            Err(AutomapError::ProtocolError(err_msg.to_string()))
        );
        TestLogHandler::new().exists_log_containing(&format!("WARN: PMPTransactor: {}", err_msg));
    }

    #[test]
    fn parse_buffer_rejects_packet_other_than_get() {
        init_test_logging();
        let router_ip = IpAddr::from_str("4.3.2.1").unwrap();
        let mut packet = PmpPacket::default();
        packet.opcode = Opcode::MapUdp;
        packet.direction = Direction::Response;
        packet.opcode_data = Box::new(MapOpcodeData::default());
        let mut buffer = [0u8; 100];
        let buflen = packet.marshal(&mut buffer).unwrap();
        let logger = Logger::new("PMPTransactor");

        let result = PmpTransactor::parse_buffer(
            &buffer[0..buflen],
            SocketAddr::new(router_ip, 5351),
            &logger,
        );

        let err_msg =
            "Unexpected PMP MapUdp response (instead of Get) from router at 4.3.2.1:5351: ignoring";
        assert_eq!(
            result,
            Err(AutomapError::ProtocolError(err_msg.to_string()))
        );
        TestLogHandler::new().exists_log_containing(&format!("WARN: PMPTransactor: {}", err_msg));
    }

    #[test]
    fn parse_buffer_rejects_unparseable_packet() {
        init_test_logging();
        let router_ip = IpAddr::from_str("4.3.2.1").unwrap();
        let buffer = [0xFFu8; 100];
        let logger = Logger::new("PMPTransactor");

        let result = PmpTransactor::parse_buffer(
            &buffer[0..2], // wayyy too short
            SocketAddr::new(router_ip, 5351),
            &logger,
        );

        let err_msg = "Unparseable packet from router at 4.3.2.1:5351: ignoring";
        assert_eq!(
            result,
            Err(AutomapError::ProtocolError(err_msg.to_string()))
        );
        TestLogHandler::new().exists_log_containing(&format!("WARN: PMPTransactor: {}", err_msg));
    }

    #[test]
    fn handle_announcement_processes_transaction_failure() {
        init_test_logging();
        let socket = UdpSocketMock::new()
            .set_read_timeout_result(Ok(()))
            .send_to_result(Ok(100))
            .recv_from_result(Err(io::Error::from(ErrorKind::ConnectionReset)), vec![]);
        let factories = Factories {
            socket_factory: Box::new(UdpSocketFactoryMock::new().make_result(Ok(socket))),
            free_port_factory: Box::new(FreePortFactoryMock::new().make_result(1234)),
        };
        let change_handler_log_arc = Arc::new(Mutex::new(vec![]));
        let change_handler_log_inner = change_handler_log_arc.clone();
        let change_handler: ChangeHandler =
            Box::new(move |change| change_handler_log_inner.lock().unwrap().push(change));
        let logger = Logger::new("test");

        PmpTransactor::handle_announcement(
            Arc::new(Mutex::new(factories)),
            SocketAddr::from_str("7.7.7.7:1234").unwrap(),
            Ipv4Addr::from_str("4.3.2.1").unwrap(),
            &change_handler,
            &ChangeHandlerConfig {
                hole_port: 2222,
                lifetime: 10000,
            },
            &logger,
        );

        let change_handler_log = change_handler_log_arc.lock().unwrap();
        assert_eq!(
            *change_handler_log,
            vec![AutomapChange::Error(AutomapError::SocketReceiveError(
                AutomapErrorCause::SocketFailure
            ))]
        );
        TestLogHandler::new().exists_log_containing("ERROR: test: Remapping after IP change failed; Node is useless: SocketReceiveError(Unknown(\"Kind(ConnectionReset)\"))");
    }

    #[test]
    fn handle_announcement_rejects_unexpected_request() {
        init_test_logging();
        let router_address = SocketAddr::from_str("7.7.7.7:1234").unwrap();
        let mut packet = PmpPacket::default();
        packet.direction = Direction::Request;
        packet.opcode = Opcode::MapTcp;
        packet.opcode_data = Box::new(MapOpcodeData::default());
        let mut buffer = [0u8; 100];
        let buflen = packet.marshal(&mut buffer).unwrap();
        let socket = UdpSocketMock::new()
            .set_read_timeout_result(Ok(()))
            .send_to_result(Ok(100))
            .recv_from_result(Ok((buflen, router_address)), buffer[0..buflen].to_vec());
        let factories = Factories {
            socket_factory: Box::new(UdpSocketFactoryMock::new().make_result(Ok(socket))),
            free_port_factory: Box::new(FreePortFactoryMock::new().make_result(1234)),
        };
        let change_handler_log_arc = Arc::new(Mutex::new(vec![]));
        let change_handler_log_inner = change_handler_log_arc.clone();
        let change_handler: ChangeHandler =
            Box::new(move |change| change_handler_log_inner.lock().unwrap().push(change));
        let logger = Logger::new("test");

        PmpTransactor::handle_announcement(
            Arc::new(Mutex::new(factories)),
            router_address,
            Ipv4Addr::from_str("4.3.2.1").unwrap(),
            &change_handler,
            &ChangeHandlerConfig {
                hole_port: 2222,
                lifetime: 10000,
            },
            &logger,
        );
    }

    #[test]
    fn handle_announcement_rejects_temporarily_unsuccessful_result_code() {
        init_test_logging();
        let router_address = SocketAddr::from_str("7.7.7.7:1234").unwrap();
        let mut packet = PmpPacket::default();
        packet.direction = Direction::Response;
        packet.opcode = Opcode::MapTcp;
        packet.result_code_opt = Some(ResultCode::OutOfResources);
        packet.opcode_data = Box::new(MapOpcodeData::default());
        let mut buffer = [0u8; 100];
        let buflen = packet.marshal(&mut buffer).unwrap();
        let socket = UdpSocketMock::new()
            .set_read_timeout_result(Ok(()))
            .send_to_result(Ok(100))
            .recv_from_result(Ok((buflen, router_address)), buffer[0..buflen].to_vec());
        let factories = Factories {
            socket_factory: Box::new(UdpSocketFactoryMock::new().make_result(Ok(socket))),
            free_port_factory: Box::new(FreePortFactoryMock::new().make_result(1234)),
        };
        let change_handler_log_arc = Arc::new(Mutex::new(vec![]));
        let change_handler_log_inner = change_handler_log_arc.clone();
        let change_handler: ChangeHandler =
            Box::new(move |change| change_handler_log_inner.lock().unwrap().push(change));
        let logger = Logger::new("test");

        PmpTransactor::handle_announcement(
            Arc::new(Mutex::new(factories)),
            router_address,
            Ipv4Addr::from_str("4.3.2.1").unwrap(),
            &change_handler,
            &ChangeHandlerConfig {
                hole_port: 2222,
                lifetime: 10000,
            },
            &logger,
        );

        let change_handler_log = change_handler_log_arc.lock().unwrap();
        let err_msg = "Remapping after IP change failed; Node is useless: OutOfResources";
        assert_eq!(
            *change_handler_log,
            vec![AutomapChange::Error(AutomapError::TemporaryMappingError(
                err_msg.to_string()
            ))]
        );
        TestLogHandler::new().exists_log_containing(&format!("ERROR: test: {}", err_msg));
    }

    #[test]
    fn handle_announcement_rejects_permanently_unsuccessful_result_code() {
        init_test_logging();
        let router_address = SocketAddr::from_str("7.7.7.7:1234").unwrap();
        let mut packet = PmpPacket::default();
        packet.direction = Direction::Response;
        packet.opcode = Opcode::MapTcp;
        packet.result_code_opt = Some(ResultCode::UnsupportedVersion);
        packet.opcode_data = Box::new(MapOpcodeData::default());
        let mut buffer = [0u8; 100];
        let buflen = packet.marshal(&mut buffer).unwrap();
        let socket = UdpSocketMock::new()
            .set_read_timeout_result(Ok(()))
            .send_to_result(Ok(100))
            .recv_from_result(Ok((buflen, router_address)), buffer[0..buflen].to_vec());
        let factories = Factories {
            socket_factory: Box::new(UdpSocketFactoryMock::new().make_result(Ok(socket))),
            free_port_factory: Box::new(FreePortFactoryMock::new().make_result(1234)),
        };
        let change_handler_log_arc = Arc::new(Mutex::new(vec![]));
        let change_handler_log_inner = change_handler_log_arc.clone();
        let change_handler: ChangeHandler =
            Box::new(move |change| change_handler_log_inner.lock().unwrap().push(change));
        let logger = Logger::new("test");

        PmpTransactor::handle_announcement(
            Arc::new(Mutex::new(factories)),
            router_address,
            Ipv4Addr::from_str("4.3.2.1").unwrap(),
            &change_handler,
            &ChangeHandlerConfig {
                hole_port: 2222,
                lifetime: 10000,
            },
            &logger,
        );

        let change_handler_log = change_handler_log_arc.lock().unwrap();
        let err_msg = "Remapping after IP change failed; Node is useless: UnsupportedVersion";
        assert_eq!(
            *change_handler_log,
            vec![AutomapChange::Error(AutomapError::PermanentMappingError(
                err_msg.to_string()
            ))]
        );
        TestLogHandler::new().exists_log_containing(&format!("ERROR: test: {}", err_msg));
    }

    #[test]
    fn remap_port_correctly_converts_lifetime_greater_than_one_second() {
        let add_mapping_params_arc = Arc::new (Mutex::new (vec![]));
        let mapping_adder = MappingAdderMock::new ()
            .add_mapping_params (&add_mapping_params_arc)
            .add_mapping_result (Err (AutomapError::Unknown));

        let result = PmpTransactor::remap_port(
            &mapping_adder,
            &Arc::new (Mutex::new (Factories::default())),
            SocketAddr::new (localhost(), 0),
            0,
            Duration::from_millis (100900),
            &Logger::new ("test"),
        );

        assert_eq! (result, Err(AutomapError::Unknown));
        let mut add_mapping_params = add_mapping_params_arc.lock().unwrap();
        let requested_lifetime: u32 = add_mapping_params.remove(0).3;
        assert_eq! (requested_lifetime, 100);
    }

    #[test]
    fn remap_port_correctly_converts_lifetime_less_than_one_second() {
        let add_mapping_params_arc = Arc::new (Mutex::new (vec![]));
        let mapping_adder = MappingAdderMock::new ()
            .add_mapping_params (&add_mapping_params_arc)
            .add_mapping_result (Err (AutomapError::Unknown));

        let result = PmpTransactor::remap_port(
            &mapping_adder,
            &Arc::new (Mutex::new (Factories::default())),
            SocketAddr::new (localhost(), 0),
            0,
            Duration::from_millis (80),
            &Logger::new ("test"),
        );

        assert_eq! (result, Err(AutomapError::Unknown));
        let mut add_mapping_params = add_mapping_params_arc.lock().unwrap();
        let requested_lifetime: u32 = add_mapping_params.remove(0).3;
        assert_eq! (requested_lifetime, 1);
    }

    #[test]
    fn remap_port_handles_temporary_mapping_failure() {
        let mapping_adder = MappingAdderMock::new ()
            .add_mapping_result (Err (AutomapError::TemporaryMappingError("NetworkFailure".to_string())));

        let result = PmpTransactor::remap_port(
            &mapping_adder,
            &Arc::new (Mutex::new (Factories::default())),
            SocketAddr::new (localhost(), 0),
            0,
            Duration::from_millis (1000),
            &Logger::new ("test"),
        );

        assert_eq! (result, Err(AutomapError::TemporaryMappingError("NetworkFailure".to_string())));
    }

    #[test]
    fn remap_port_handles_permanent_mapping_failure() {
        let mapping_transactor = MappingAdderMock::new ()
            .add_mapping_result (Err (AutomapError::PermanentMappingError("MalformedRequest".to_string())));

        let result = PmpTransactor::remap_port(
            &mapping_transactor,
            &Arc::new (Mutex::new (Factories::default())),
            SocketAddr::new (localhost(), 0),
            0,
            Duration::from_millis (1000),
            &Logger::new ("test"),
        );

        assert_eq! (result, Err(AutomapError::PermanentMappingError("MalformedRequest".to_string())));
    }

    fn make_subject(socket_factory: UdpSocketFactoryMock) -> PmpTransactor {
        let mut subject = PmpTransactor::default();
        let mut factories = Factories::default();
        factories.socket_factory = Box::new(socket_factory);
        factories.free_port_factory = Box::new(FreePortFactoryMock::new().make_result(5566));
        subject.factories_arc = Arc::new(Mutex::new(factories));
        subject
    }

    fn make_request(opcode: Opcode, opcode_data: Box<dyn PmpOpcodeData>) -> PmpPacket {
        PmpPacket {
            direction: Direction::Request,
            opcode,
            result_code_opt: None,
            opcode_data,
        }
    }

    fn make_get_request() -> Box<GetOpcodeData> {
        Box::new(GetOpcodeData {
            epoch_opt: None,
            external_ip_address_opt: None,
        })
    }

    fn make_map_request(port: u16, lifetime: u32) -> Box<MapOpcodeData> {
        Box::new(MapOpcodeData {
            epoch_opt: None,
            internal_port: port,
            external_port: port,
            lifetime,
        })
    }

    fn make_response(
        opcode: Opcode,
        result_code: ResultCode,
        opcode_data: Box<dyn PmpOpcodeData>,
    ) -> PmpPacket {
        PmpPacket {
            direction: Direction::Response,
            opcode,
            result_code_opt: Some(result_code),
            opcode_data,
        }
    }

    fn make_get_response(epoch_time: u32, external_ip_address: Ipv4Addr) -> Box<GetOpcodeData> {
        Box::new(GetOpcodeData {
            epoch_opt: Some(epoch_time),
            external_ip_address_opt: Some(external_ip_address),
        })
    }

    fn make_map_response(epoch_time: u32, port: u16, lifetime: u32) -> Box<MapOpcodeData> {
        Box::new(MapOpcodeData {
            epoch_opt: Some(epoch_time),
            internal_port: port,
            external_port: port,
            lifetime,
        })
    }
}

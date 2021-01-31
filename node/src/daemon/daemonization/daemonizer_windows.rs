// Copyright (c) 2019-2021, MASQ (https://masq.ai). All rights reserved.

#![cfg (target_os = "windows")]

use crate::daemon::daemonization::daemonizer::{DaemonizerError, DaemonHandle, DaemonHandleFactory, DaemonStarter};
use windows_service::{Error, service_dispatcher, service_control_handler};
use std::ffi::{OsString};
use windows_service::service_control_handler::{ServiceStatusHandle, ServiceControlHandlerResult};
use windows_service::service::{ServiceControl, ServiceStatus, ServiceType, ServiceState, ServiceControlAccept, ServiceExitCode};
use std::time::Duration;

define_windows_service!(masqd, masqd_fn);

pub fn platform_daemonize<F: FnOnce() -> Result<(), DaemonizerError>>(daemon_starter: Box<dyn DaemonStarter>, daemon_code: F) -> Result<(), DaemonizerError> {
    daemon_starter.start();
    daemon_code ()?;
    Ok(())
}

fn masqd_fn (arguments: Vec<OsString>) {
    unimplemented!()
}

pub struct DaemonStarterReal {

}

impl DaemonStarter for DaemonStarterReal {
    fn start(&self) {
        unimplemented!()
    }
}

impl DaemonStarterReal {
    pub fn new () -> Self {
        Self {

        }
    }
}

pub struct DaemonHandleReal {
    status_handle: Box<dyn StatusHandleWrapper>
}

impl DaemonHandle for DaemonHandleReal {
    fn signal_termination(&self) {
        self.status_handle.set_service_status(
            ServiceStatus {
                service_type: ServiceType::OWN_PROCESS,
                current_state: ServiceState::StopPending,
                controls_accepted: ServiceControlAccept::STOP,
                exit_code: ServiceExitCode::Win32(1),
                checkpoint: 0,
                wait_hint: Duration::from_millis(1000),
                process_id: None
            }
        );
    }

    fn finish_termination(&self) {
        self.status_handle.set_service_status(
            ServiceStatus {
                service_type: ServiceType::OWN_PROCESS,
                current_state: ServiceState::Stopped,
                controls_accepted: ServiceControlAccept::STOP,
                exit_code: ServiceExitCode::Win32(1),
                checkpoint: 0,
                wait_hint: Duration::from_millis(1000),
                process_id: None
            }
        );
    }
}

impl DaemonHandleReal {
    pub fn new(status_handle: Box<dyn StatusHandleWrapper>) -> Self {
        Self { status_handle }
    }
}

pub struct DaemonHandleFactoryReal {
    service_registrar: Box<dyn ServiceRegistrar>,
}

impl DaemonHandleFactory for DaemonHandleFactoryReal {
    fn make(&self) -> Result<Box<dyn DaemonHandle>, DaemonizerError> {
        let status_handle = match self.service_registrar.register ("masqd") {
            Ok(ssh) => ssh,
            Err (e) => unimplemented!(),
        };
        status_handle.set_service_status(ServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            current_state: ServiceState::Running,
            controls_accepted: ServiceControlAccept::STOP,
            exit_code: ServiceExitCode::Win32(1),
            checkpoint: 0,
            wait_hint: Duration::from_millis(1000),
            process_id: None
        });
        Ok(Box::new (DaemonHandleReal::new(status_handle)))
    }
}

impl DaemonHandleFactoryReal {
    pub fn new() -> Self {
        Self {
            service_registrar: Box::new (ServiceRegistrarReal::new()),
        }
    }
}

trait ServiceRegistrar {
    fn register(&self, service_name: &str) -> Result<Box<dyn StatusHandleWrapper>, windows_service::Error>;
    fn handle_event (&self, control: ServiceControl) -> ServiceControlHandlerResult;
}

struct ServiceRegistrarReal {

}

impl ServiceRegistrar for ServiceRegistrarReal {
    fn register(&self, service_name: &str) -> Result<Box<dyn StatusHandleWrapper>, windows_service::Error> {
        unimplemented!()
    }

    fn handle_event(&self, control: ServiceControl) -> ServiceControlHandlerResult {
        unimplemented!()
    }
}

impl ServiceRegistrarReal {
    fn new () -> Self {
        Self {}
    }
}

pub trait StatusHandleWrapper {
    fn set_service_status(&self, service_status: ServiceStatus) -> Result<(), windows_service::Error>;
}

struct StatusHandleReal {
    status_handle: ServiceStatusHandle,
}

impl StatusHandleWrapper for StatusHandleReal {
    fn set_service_status(&self, service_status: ServiceStatus) -> Result<(), windows_service::Error> {
        self.status_handle.set_service_status(service_status)
    }
}

impl StatusHandleReal {
    fn new (status_handle: ServiceStatusHandle) -> Self {
        Self {status_handle}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::sync::{Mutex, Arc};
    use std::mem::{size_of, transmute};
    use std::alloc::{Layout, alloc};
    use windows_service::service::{ServiceType, ServiceState, ServiceControlAccept, ServiceExitCode};
    use std::time::Duration;

    struct DaemonStarterMock {
        start_params: Arc<Mutex<Vec<()>>>
    }

    impl DaemonStarter for DaemonStarterMock {
        fn start(&self) {
            self.start_params.lock().unwrap().push (());
        }
    }

    impl DaemonStarterMock {
        fn new () -> Self {
            Self {
                start_params: Arc::new(Mutex::new(vec![]))
            }
        }

        fn start_params (mut self, params: &Arc<Mutex<Vec<()>>>) -> Self {
            self.start_params = params.clone();
            self
        }
    }

    #[derive (Clone, PartialEq, Debug)]
    struct DaemonHandleMock {

    }

    impl DaemonHandle for DaemonHandleMock {
        fn signal_termination(&self) {
            unimplemented!()
        }

        fn finish_termination(&self) {
            unimplemented!()
        }
    }

    impl DaemonHandleMock {
        fn new () -> Self {
            Self {}
        }
    }

    struct ServiceRegistrarMock {
        register_params: Arc<Mutex<Vec<String>>>,
        register_results: RefCell<Vec<Result<Box<dyn StatusHandleWrapper>, Error>>>,
        handle_event_params: Arc<Mutex<Vec<ServiceControl>>>,
        handle_event_results: RefCell<Vec<ServiceControlHandlerResult>>,
    }

    impl ServiceRegistrar for ServiceRegistrarMock {
        fn register(&self, service_name: &str) -> Result<Box<dyn StatusHandleWrapper>, Error> {
            self.register_params.lock().unwrap().push (service_name.to_string());
            self.register_results.borrow_mut().remove(0)
        }

        fn handle_event(&self, control: ServiceControl) -> ServiceControlHandlerResult {
            self.handle_event_params.lock().unwrap().push (control);
            self.handle_event_results.borrow_mut().remove (0)
        }
    }

    impl ServiceRegistrarMock {
        fn new () -> Self {
            Self {
                register_params: Arc::new(Mutex::new(vec![])),
                register_results: RefCell::new(vec![]),
                handle_event_params: Arc::new(Mutex::new(vec![])),
                handle_event_results: RefCell::new(vec![])
            }
        }

        fn register_params (mut self, params: &Arc<Mutex<Vec<String>>>) -> Self {
            self.register_params = params.clone();
            self
        }

        fn register_result (self, result: Result<Box<dyn StatusHandleWrapper>, Error>) -> Self {
            self.register_results.borrow_mut().push (result);
            self
        }

        fn handle_event_params (mut self, params: &Arc<Mutex<Vec<ServiceControl>>>) -> Self {
            self.handle_event_params = params.clone();
            self
        }

        fn handle_event_result (self, result: ServiceControlHandlerResult) -> Self {
            self.handle_event_results.borrow_mut().push (result);
            self
        }
    }

    struct StatusHandleMock {
        set_service_status_params: Arc<Mutex<Vec<ServiceStatus>>>,
        set_service_status_results: RefCell<Vec<Result<(), windows_service::Error>>>
    }

    impl StatusHandleWrapper for StatusHandleMock {
        fn set_service_status(&self, service_status: ServiceStatus) -> Result<(), windows_service::Error> {
            self.set_service_status_params.lock().unwrap().push (service_status);
            self.set_service_status_results.borrow_mut().remove(0)
        }
    }

    impl StatusHandleMock {
        fn new () -> Self {
            Self{
                set_service_status_params: Arc::new(Mutex::new(vec![])),
                set_service_status_results: RefCell::new(vec![]),
            }
        }

        fn set_service_status_params(mut self, params: &Arc<Mutex<Vec<ServiceStatus>>>) -> Self {
            self.set_service_status_params = params.clone();
            self
        }

        fn set_service_status_result (self, result: Result<(), windows_service::Error>) -> Self {
            self.set_service_status_results.borrow_mut().push (result);
            self
        }
    }

    #[test]
    fn platform_daemonize_works() {
        let daemon_code_ran_outer = Arc::new(Mutex::new(vec![]));
        let daemon_code_ran_inner = daemon_code_ran_outer.clone();
        let start_params_arc = Arc::new (Mutex::new (vec![]));
        let daemon_starter = DaemonStarterMock::new()
            .start_params (&start_params_arc);

        platform_daemonize(Box::new (daemon_starter), move || {
            daemon_code_ran_inner.lock().unwrap().push (());
            Ok(())
        });

        let start_params = start_params_arc.lock().unwrap();
        assert_eq! (*start_params, vec![()]);
        let daemon_code_ran = daemon_code_ran_outer.lock().unwrap();
        assert_eq! (*daemon_code_ran, vec![()]);
    }

    #[test]
    fn handle_factory_constructor_works () {
        let set_service_status_params_arc = Arc::new (Mutex::new (vec![]));
        let status_handle = StatusHandleMock::new()
            .set_service_status_params (&set_service_status_params_arc)
            .set_service_status_result (Ok(()));
        let daemon_handle = DaemonHandleMock::new();
        let register_params_arc = Arc::new (Mutex::new (vec![]));
        let service_registrar = ServiceRegistrarMock::new()
            .register_params (&register_params_arc)
            .register_result (Ok(Box::new (status_handle)));
        let mut subject = DaemonHandleFactoryReal::new();
        subject.service_registrar = Box::new (service_registrar);

        let result = subject.make();

        assert_eq! (result.is_ok(), true);
        let register_params = register_params_arc.lock().unwrap();
        assert_eq! (*register_params, vec!["masqd".to_string()]);
        let set_service_status_params = set_service_status_params_arc.lock().unwrap();
        assert_eq! (*set_service_status_params, vec![
            ServiceStatus {
                service_type: ServiceType::OWN_PROCESS,
                current_state: ServiceState::Running,
                controls_accepted: ServiceControlAccept::STOP,
                exit_code: ServiceExitCode::Win32(1),
                checkpoint: 0,
                wait_hint: Duration::from_millis(1000),
                process_id: None
            }
        ])
    }

    #[test]
    fn daemon_handle_signals_termination() {
        let set_service_status_params_arc = Arc::new(Mutex::new(vec![]));
        let status_handle = StatusHandleMock::new()
            .set_service_status_params (&set_service_status_params_arc)
            .set_service_status_result (Ok(()));
        let mut subject = DaemonHandleReal::new(Box::new (status_handle));

        subject.signal_termination();

        let set_service_status_params = set_service_status_params_arc.lock().unwrap();
        assert_eq! (*set_service_status_params, vec![
            ServiceStatus {
                service_type: ServiceType::OWN_PROCESS,
                current_state: ServiceState::StopPending,
                controls_accepted: ServiceControlAccept::STOP,
                exit_code: ServiceExitCode::Win32(1),
                checkpoint: 0,
                wait_hint: Duration::from_millis(1000),
                process_id: None
            }
        ])
    }

    #[test]
    fn daemon_handle_finishes_termination() {
        let set_service_status_params_arc = Arc::new(Mutex::new(vec![]));
        let status_handle = StatusHandleMock::new()
            .set_service_status_params (&set_service_status_params_arc)
            .set_service_status_result(Ok(()));
        let mut subject = DaemonHandleReal::new(Box::new (status_handle));

        subject.finish_termination();

        let set_service_status_params = set_service_status_params_arc.lock().unwrap();
        assert_eq! (*set_service_status_params, vec![
            ServiceStatus {
                service_type: ServiceType::OWN_PROCESS,
                current_state: ServiceState::Stopped,
                controls_accepted: ServiceControlAccept::STOP,
                exit_code: ServiceExitCode::Win32(1),
                checkpoint: 0,
                wait_hint: Duration::from_millis(1000),
                process_id: None
            }
        ])
    }

    fn jackass_service_status_handle() -> ServiceStatusHandle {
        let layout = Layout::new::<ServiceStatusHandle>();
        let ptr = unsafe {alloc(layout)};
        unsafe {transmute::<*mut u8, ServiceStatusHandle>(ptr)}
    }
}

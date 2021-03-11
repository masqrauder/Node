// Copyright (c) 2019-2021, MASQ (https://masq.ai). All rights reserved.

#![cfg(target_os = "windows")]

use crate::daemon::daemonization::daemonizer::{
    DaemonHandle, DaemonHandleFactory, DaemonizerError,
};
use lazy_static::lazy_static;
use std::ffi::OsString;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use windows_service::service::{
    ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler;

type DaemonCode = dyn FnOnce() -> Result<(), DaemonizerError>;

static mut DAEMON_CODE: [Option<Box<DaemonCode>>; 1] = [None];

lazy_static! {
    static ref DAEMON_CODE_BANK_MONITOR: Arc<Mutex<()>> = Arc::new(Mutex::new(()));
}

define_windows_service!(masqd, masqd_fn);

fn service_status(current_state: ServiceState) -> ServiceStatus {
    ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state,
        controls_accepted: ServiceControlAccept::STOP,
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::from_millis(1000),
        process_id: None,
    }
}

pub fn platform_daemonize<F: FnOnce() -> Result<(), DaemonizerError> + 'static>(
    daemon_code: F,
) -> Result<(), DaemonizerError> {
    set_code(Box::new(daemon_code));
    let status_handle = match service_control_handler::register("masqd", event_handler) {
        Ok(sh) => sh,
        Err(e) => unimplemented!("{:?}", e),
    };
    status_handle.set_service_status(service_status(ServiceState::Running));
    Ok(())
}

fn masqd_fn(arguments: Vec<OsString>) {
    let daemon_code = take_code();

    unimplemented!()
}

fn set_code(code: Box<DaemonCode>) {
    unsafe {
        let _hold_open = DAEMON_CODE_BANK_MONITOR.lock();
        if DAEMON_CODE[0].is_some() {
            panic!("Daemon code is already set");
        }
        let _ = DAEMON_CODE[0].replace(code);
    }
}

fn take_code() -> Box<DaemonCode> {
    unsafe {
        let _hold_open = DAEMON_CODE_BANK_MONITOR.lock();
        DAEMON_CODE[0].take().expect("Daemon code isn't set")
    }
}

pub struct DaemonHandleFactoryReal {}

impl DaemonHandleFactory for DaemonHandleFactoryReal {
    fn make(&self) -> Result<Box<dyn DaemonHandle>, DaemonizerError> {
        unimplemented!()
    }
}

impl DaemonHandleFactoryReal {
    pub fn new() -> Self {
        Self {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use masq_lib::test_utils::environment_guard::EnvironmentGuard;

    #[test]
    fn daemon_code_bank_works() {
        let _serialized = EnvironmentGuard::new();
        let target_arc = Arc::new(Mutex::new(vec![]));
        let target_inner = target_arc.clone();
        let daemon_code = move || {
            target_inner.lock().unwrap().push(1000);
            Ok(())
        };

        set_code(Box::new(daemon_code));
        let actual = take_code();

        assert_eq!(actual(), Ok(()));
        let target = target_arc.lock().unwrap();
        assert_eq!(*target, vec![1000]);
    }
}

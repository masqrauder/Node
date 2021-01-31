// Copyright (c) 2019-2021, MASQ (https://masq.ai). All rights reserved.

#![cfg (target_os = "windows")]

use windows_service::{Error, service_dispatcher, service_control_handler};
use std::ffi::{OsString};
use windows_service::service_control_handler::{ServiceStatusHandle, ServiceControlHandlerResult};
use windows_service::service::{ServiceControl, ServiceStatus, ServiceType, ServiceState, ServiceControlAccept, ServiceExitCode};
use std::time::Duration;
use crate::daemon::daemonization::daemonizer::{DaemonizerError, DaemonHandleFactory, DaemonHandle};
use lazy_static::lazy_static;
use std::sync::{Mutex, Arc};
use std::collections::HashMap;
use rand::{thread_rng, RngCore};
use itertools::Itertools;

type DaemonCode = dyn FnOnce() -> Result<(), DaemonizerError>;

static mut DAEMON_CODE_BANK: [Option<Box<DaemonCode>>; 16] = [
    None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None
];

lazy_static! {
    static ref DAEMON_CODE_BANK_MONITOR: Arc<Mutex<()>> = Arc::new (Mutex::new (()));
}

define_windows_service!(masqd, masqd_fn);

pub fn platform_daemonize<F: FnOnce() -> Result<(), DaemonizerError>>(daemon_code: F) -> Result<(), DaemonizerError> {
    let handle = add_code (Box::new (daemon_code));
    Ok(())
}

fn masqd_fn (arguments: Vec<OsString>) {
    unimplemented!()
}

fn add_code(code: Box<DaemonCode>) -> usize {
    unsafe {
        let _hold_open = DAEMON_CODE_BANK_MONITOR.lock();
        let handle = match DAEMON_CODE_BANK.iter().find_position(|element| element.is_none()) {
            None => panic! ("Too much daemon code in the code bank"),
            Some ((idx, _)) => idx,
        };
        DAEMON_CODE_BANK[handle] = Some (code);
        handle
    }
}

fn get_code(handle: usize) -> Option<Box<DaemonCode>> {
    unsafe {
        let _hold_open = DAEMON_CODE_BANK_MONITOR.lock();
        DAEMON_CODE_BANK[handle].take()
    }
}

pub struct DaemonHandleFactoryReal {

}

impl DaemonHandleFactory for DaemonHandleFactoryReal {
    fn make(&self) -> Result<Box<dyn DaemonHandle>, DaemonizerError> {
        unimplemented!()
    }
}

impl DaemonHandleFactoryReal {
    pub fn new () -> Self {
        Self {

        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use masq_lib::test_utils::environment_guard::EnvironmentGuard;

    #[test]
    fn daemon_code_bank_works() {
        let _serialized = EnvironmentGuard::new();
        let target_arc = Arc::new (Mutex::new (vec![]));
        let target_1000 = target_arc.clone();
        let daemon_code_1000 = move || {target_1000.lock().unwrap().push (1000); Ok(())};
        let target_2000 = target_arc.clone();
        let daemon_code_2000 = move || {target_2000.lock().unwrap().push (2000); Ok(())};
        let target_3000 = target_arc.clone();
        let daemon_code_3000 = move || {target_3000.lock().unwrap().push (3000); Ok(())};

        let handle_1000 = add_code (Box::new (daemon_code_1000));
        let handle_2000 = add_code (Box::new (daemon_code_2000));
        let handle_3000 = add_code (Box::new (daemon_code_3000));

        let actual_1000 = get_code (handle_1000).unwrap();
        let actual_2000 = get_code (handle_2000).unwrap();
        let actual_3000 = get_code (handle_3000).unwrap();

        assert_eq! (actual_2000(), Ok(()));
        assert_eq! (actual_1000(), Ok(()));
        assert_eq! (actual_3000(), Ok(()));
        let target = target_arc.lock().unwrap();
        assert_eq! (*target, vec![2000, 1000, 3000]);
    }

    #[test]
    fn daemon_code_bank_handles_missing_code() {
        let _serialized = EnvironmentGuard::new();

        let result = get_code (15);

        assert_eq! (result.is_none(), true);
    }
}

// Copyright (c) 2019-2021, MASQ (https://masq.ai). All rights reserved.

#![cfg (target_os = "windows")]

use crate::daemon::daemonization::daemonizer::{DaemonizerError, DaemonHandle, DaemonHandleFactory};
use windows_service::{Error};
use std::ffi::{OsStr, OsString};
use windows_service::service_control_handler::{ServiceStatusHandle, ServiceControlHandlerResult};
use windows_service::service::{ServiceControl, ServiceStatus};

define_windows_service!(masqd, masqd_fn);

pub fn platform_daemonize<F: FnOnce() -> Result<(), DaemonizerError>>(daemon_code: F) -> Result<(), DaemonizerError> {
    daemon_code ()?;
    Ok(())
}

fn masqd_fn (arguments: Vec<OsString>) {
    unimplemented!()
}

pub struct DaemonHandleReal {

}

impl DaemonHandle for DaemonHandleReal {
    fn signal_termination(&self) {
        unimplemented!()
    }

    fn finish_termination(&self) {
        unimplemented!()
    }
}

impl DaemonHandleReal {
    pub fn new() -> Self {
        unimplemented!()
    }
}

pub struct DaemonHandleFactoryReal {
    service_registrar: Box<dyn ServiceRegistrar>,
}

impl DaemonHandleFactory for DaemonHandleFactoryReal {
    fn make(&self) -> Result<Box<dyn DaemonHandle>, DaemonizerError> {
        unimplemented!()
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
    fn register(&self, service_name: &str) -> Result<ServiceStatusHandle, windows_service::Error>;
    fn handle_event (&self, control: ServiceControl) -> ServiceControlHandlerResult;
}

struct ServiceRegistrarReal {

}

impl ServiceRegistrar for ServiceRegistrarReal {
    fn register(&self, service_name: &str) -> Result<ServiceStatusHandle, Error> {
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

trait StatusHandleWrapper {
    fn set_service_status(&self, service_status: ServiceStatus) -> Result<(), Error>;
}

struct StatusHandleReal {
    status_handle: ServiceStatusHandle,
}

impl StatusHandleWrapper for StatusHandleReal {
    fn set_service_status(&self, service_status: ServiceStatus) -> Result<(), Error> {
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

    #[test]
    fn nothing () {

    }
}

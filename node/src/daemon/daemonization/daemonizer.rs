// Copyright (c) 2019-2021, MASQ (https://masq.ai). All rights reserved.

#[derive (PartialEq, Clone, Debug)]
pub enum DaemonizerError {
    Other(String)
}

pub trait Daemonizer {
    fn daemonize(&self) -> Result<DaemonHandle, DaemonizerError>;
}

pub struct DaemonizerReal {
}

impl Daemonizer for DaemonizerReal {
    fn daemonize(&self) -> Result<DaemonHandle, DaemonizerError> {
        #[cfg(target_os = "linux")]
        return crate::daemon::daemonization::daemonizer_linux::daemonize();

        #[cfg(target_os = "macos")]
        return crate::daemon::daemonization::daemonizer_macos::daemonize();

        #[cfg(target_os = "windows")]
        return crate::daemon::daemonization::daemonizer_windows::daemonize()
    }
}

impl DaemonizerReal {
    pub fn new() -> Self {
        Self{}
    }
}

pub trait DaemonHandle {
    fn signal_termination(&self);
    fn finish_termination(&self);
}

// Copyright (c) 2019-2021, MASQ (https://masq.ai). All rights reserved.

#[derive(PartialEq, Clone, Debug)]
pub enum DaemonizerError {
    Other(String),
}

pub trait DaemonStarter {
    fn start(&self);
}

pub trait DaemonHandle {
    fn signal_termination(&self);
    fn finish_termination(&self);
}

pub trait DaemonHandleFactory {
    fn make(&self) -> Result<Box<dyn DaemonHandle>, DaemonizerError>;
}

pub fn daemonize<F: FnOnce() -> Result<(), DaemonizerError> + 'static>(
    daemon_code: F,
) -> Result<(), DaemonizerError> {
    #[cfg(target_os = "linux")]
    unimplemented!();

    #[cfg(target_os = "macos")]
    unimplemented!();

    #[cfg(target_os = "windows")]
    return crate::daemon::daemonization::daemonizer_windows::platform_daemonize(daemon_code);
}

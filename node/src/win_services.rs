// Copyright (c) 2019-2021, MASQ (https://masq.ai) and/or its affiliates. All rights reserved.

use crate::sub_lib::logger::Logger;
use std::env::var;
use std::ffi::OsString;
use std::path::{PathBuf, Path};
use std::process::{Child, Command};
use windows_service::{service_dispatcher, Result};

#[cfg(target_os = "windows")]
define_windows_service!(boot_win_masqd, win_masq_deamon); //boot_win_masqd is bound with Windows services,

#[allow(dead_code)]
#[cfg(target_os = "windows")]
fn win_masq_deamon(_arguments: Vec<OsString>) {
    spawn_child(initiating_command()).wait(); //arranged for mocking
}

fn spawn_child(command: &mut Command) -> Child {
    match command.spawn() {
        Ok(ch) => {
            info!(
                Logger::new("Deamon"),
                "Successfully launched in the background"
            );
            ch
        }
        Err(error) => panic!(
            "Starting MASQNode failed; deamon has stayed down.\n
            ERROR: {}",
            error
        ),
    }
}

fn initiating_command() -> &'static mut Command {
    let path = path_from_environment();
    let command = Command::new(path).arg("--initialization");
    command
}

fn path_from_environment() -> PathBuf {
    //this will be probably dealt differently once we incorporate a real installation process
    let os_string_key = OsString::from("MASQ_DEAMON_PATH".to_string());
    let path = PathBuf::from(match var(os_string_key) {
        Ok(path) => path,
        Err(error) => panic!(
            "MASQ: the path of the Deamon in EV is not set properly: {}",
            error
        ),
    };
    path.join("MASQNode.exe")
}

#[cfg(target_os = "windows")]
fn main() -> Result<()> {
    // Register generated `ffi_service_main` with the system and start the service, blocking
    // this thread until the service is stopped.
    service_dispatcher::start("masqd", boot_win_masqd)?;
    Ok(())
}

#[cfg(test)]
#[cfg(target_os = "windows")]
mod tests {
    use super::*;
    use crate::test_utils::logging::{init_test_logging, TestLogHandler};
    use crossbeam_channel::unbounded;
    use std::env::{current_dir, set_var};
    use std::process::Stdio;
    use std::str::Chars;
    use std::time::Duration;
    use std::{env, thread};
    use std::io::{Error, ErrorKind, BufReader, stdout, BufRead};

    #[test]
    fn win_masq_deamon_produces_starting_process() {
        init_test_logging();
        let build_path = deamon_dev_directory();
        let os_string_key = OsString::from("MASQ_DEAMON_PATH");
        set_var(os_string_key, build_path);
        let (tx, rc) = unbounded();
        let (tx_back, rc_back) = unbounded();

        let _ = thread::spawn(move || {
            let child = initiating_command().stdout(Stdio::piped());
            let mut process_handle = spawn_child(child);
            let process_talk = process_handle.stdout
                .ok_or_else(||Error::new(ErrorKind::Other,"Could not capture standard output.")).unwrap();

            loop {
                match rc.try_recv() {
                    Ok(b) if b == true => break,
                    Err(_) => continue,
                    _ => panic!("Constant error in this code"),
                }
            }

            let reader = BufReader::new(process_talk);
            let mut string_message = String::new();
            reader
                .lines()
                .filter_map(|line| line.ok())
                .for_each(|line| string_message.push_str(&format!("{}", line)));

            tx_back.try_send(string_message).unwrap();
            process_handle.kill().unwrap();
        });
        thread::sleep(Duration::from_millis(1000));

        tx.send(true).unwrap();
        let result = loop {
            match rc_back.try_recv() {
                Ok(mail) => break format!("{:?}", mail),
                Err(e) => continue,
            }
        };
        TestLogHandler::new()
            .exists_log_containing("Deamon: Successfully launched in the background");
        assert_eq!(result, "blah")
    }

    // #[test]
    // fn win_masq_deamon_produces_starting_process() {
    //     init_test_logging();
    //     let build_path = deamon_dev_directory();
    //     let os_string_key = OsString::from("MASQ_DEAMON_PATH");
    //     set_var(os_string_key, build_path);
    //     let (tx, rc) = unbounded();
    //     let (tx_back, rc_back) = unbounded();
    //     let child = initiating_command().stdout(Stdio::piped());
    //
    //     let deamon_handle = spawn_child(child).wait_with_output().unwrap();
    //
    //     thread::sleep(Duration::from_millis(1000));
    //
    //     let eve_dropper = String::from_utf8(deamon_handle.stdout).unwrap();
    //
    //     deamon_handle.kill().unwrap();
    //
    //     thread::sleep(Duration::from_millis(1000));
    //
    //     TestLogHandler::new()
    //         .exists_log_containing("Deamon: Successfully launched in the background");
    //     //   assert_eq!(result, "blah")
    // }

    fn deamon_dev_directory() -> String {
        let env_scan = env::args().next().unwrap();
        let mut iter = env_scan.split("\\");
        let index = iter.position(|s| s == "release" || s == "debug").unwrap();
        //rewriting the iterator to return to the beginning; split does not work reversely
        let iter = env_scan.split("\\");
        let iter_after_cut = iter.take(index + 1);
        let mut str_path = String::new();
        iter_after_cut.for_each(|pc| str_path.push_str(&format!("{}\\", pc)));
        str_path
    }
}

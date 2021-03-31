// Copyright (c) 2019-2021, MASQ (https://masq.ai) and/or its affiliates. All rights reserved.

use masq_cli_lib::command_factory::CommandFactoryError::{CommandSyntax, UnrecognizedSubcommand};
use masq_cli_lib::command_factory::{CommandFactory, CommandFactoryReal};
use masq_cli_lib::command_processor::{
    CommandProcessor, CommandProcessorFactory, CommandProcessorFactoryReal,
};
use masq_cli_lib::communications::broadcast_handler::StreamFactoryReal;
use masq_cli_lib::interactive_mode::go_interactive;
use masq_cli_lib::terminal_interface::{InterfaceReal, TerminalInterfaceFactory};
use masq_lib::command;
use masq_lib::command::{Command, StdStreams};
use masq_lib::short_writeln;
use std::io;

fn main() {
    let mut streams: StdStreams<'_> = StdStreams {
        stdin: &mut io::stdin(),
        stdout: &mut io::stdout(),
        stderr: &mut io::stderr(),
    };

    let args: Vec<String> = std::env::args().collect();
    let streams_ref: &mut StdStreams<'_> = &mut streams;
    let exit_code = Main::new().go(streams_ref, &args);
    ::std::process::exit(i32::from(exit_code));
}

struct Main {
    command_factory: Box<dyn CommandFactory>,
    processor_factory: Box<dyn CommandProcessorFactory>,
    terminal_interface_factory: Box<dyn TerminalInterfaceFactory>,
}

impl Main {
    pub fn new() -> Self {
        Self {
            command_factory: Box::new(CommandFactoryReal::new()),
            processor_factory: Box::new(CommandProcessorFactoryReal {}),
            terminal_interface_factory: Box::new(InterfaceReal {}),
        }
    }
    fn extract_subcommand(args: &[String]) -> Option<Vec<String>> {
        let args_vec: Vec<String> = args.to_vec();
        for idx in 1..args_vec.len() {
            let one = &args_vec[idx - 1];
            let two = &args_vec[idx];
            if !one.starts_with("--") && !two.starts_with("--") {
                return Some(args_vec.into_iter().skip(idx).collect());
            }
        }
        None
    }
}

impl command::Command for Main {
    fn go(&mut self, streams: &mut StdStreams<'_>, args: &[String]) -> u8 {
        let broadcast_stream_factory = StreamFactoryReal::new();
        let interface = match self.terminal_interface_factory.make() {
            Ok(interface) => interface,
            Err(error) => {
                short_writeln!(streams.stderr, "Terminal interface: {}", error);
                return 1;
            }
        };
        let mut command_processor = match self.processor_factory.make(
            Box::new(interface),
            Box::new(broadcast_stream_factory),
            args,
        ) {
            Ok(processor) => processor,
            Err(e) => {
                short_writeln!(streams.stderr, "Can't connect to Daemon or Node ({:?}). Probably this means the Daemon isn't running.", e);
                return 1;
            }
        };
        let result = match Self::extract_subcommand(args) {
            Some(command_parts) => {
                match handle_command_common(
                    &self.command_factory,
                    &mut command_processor,
                    command_parts,
                    streams.stderr,
                ) {
                    Ok(_) => 0,
                    Err(_) => 1,
                }
            }
            None => go_interactive(
                Box::new(handle_command_common),
                &self.command_factory,
                &mut command_processor,
                streams,
            ),
        };
        command_processor.close();
        result
    }
}

fn handle_command_common(
    command_factory: &Box<dyn CommandFactory>,
    processor: &mut Box<dyn CommandProcessor>,
    command_parts: Vec<String>,
    stderr: &mut (dyn io::Write + Send),
) -> Result<(), ()> {
    let command = match command_factory.make(command_parts) {
        Ok(c) => c,
        Err(UnrecognizedSubcommand(msg)) => {
            short_writeln!(stderr, "Unrecognized command: '{}'", msg);
            return Err(());
        }
        Err(CommandSyntax(msg)) => {
            short_writeln!(stderr, "{}", msg);
            return Err(());
        }
    };
    if let Err(e) = processor.process(command) {
        short_writeln!(stderr, "{}", e);
        Err(())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use masq_cli_lib::command_context::CommandContext;
    use masq_cli_lib::command_context::ContextError::Other;
    use masq_cli_lib::command_factory::CommandFactoryError;
    use masq_cli_lib::commands::commands_common;
    use masq_cli_lib::commands::commands_common::CommandError;
    use masq_cli_lib::commands::commands_common::CommandError::Transmission;
    use masq_cli_lib::line_reader::{TerminalEvent, TerminalReal};
    use masq_cli_lib::terminal_interface::TerminalWrapper;
    use masq_cli_lib::test_utils::mocks::{
        CommandContextMock, CommandFactoryMock, CommandProcessorFactoryMock, CommandProcessorMock,
        InterfaceMock, InterfaceRawMock, MockCommand, TerminalPassiveMock,
    };
    use masq_lib::intentionally_blank;
    use masq_lib::messages::ToMessageBody;
    use masq_lib::messages::UiShutdownRequest;
    use masq_lib::test_utils::fake_stream_holder::FakeStreamHolder;
    use std::sync::{Arc, Mutex};

    #[test]
    fn noninteractive_mode_works_when_everything_is_copacetic() {
        let command = MockCommand::new(UiShutdownRequest {}.tmb(1));
        let c_make_params_arc = Arc::new(Mutex::new(vec![]));
        let command_factory = CommandFactoryMock::new()
            .make_params(&c_make_params_arc)
            .make_result(Ok(Box::new(command)));
        let interface = InterfaceMock::new()
            .make_result(Ok(TerminalReal::new(Box::new(InterfaceRawMock::new()))));
        let process_params_arc = Arc::new(Mutex::new(vec![]));
        let processor = CommandProcessorMock::new()
            .process_params(&process_params_arc)
            .process_result(Ok(()));
        let p_make_params_arc = Arc::new(Mutex::new(vec![]));
        let processor_factory = CommandProcessorFactoryMock::new()
            .make_params(&p_make_params_arc)
            .make_result(Ok(Box::new(processor)));
        let mut subject = Main {
            command_factory: Box::new(command_factory),
            processor_factory: Box::new(processor_factory),
            terminal_interface_factory: Box::new(interface),
        };

        let result = subject.go(
            &mut FakeStreamHolder::new().streams(),
            &[
                "command".to_string(),
                "--param1".to_string(),
                "value1".to_string(),
                "--param2".to_string(),
                "value2".to_string(),
                "subcommand".to_string(),
                "--param3".to_string(),
                "value3".to_string(),
                "param4".to_string(),
                "param5".to_string(),
            ],
        );

        assert_eq!(result, 0);
        let c_make_params = c_make_params_arc.lock().unwrap();
        assert_eq!(
            *c_make_params,
            vec![vec![
                "subcommand".to_string(),
                "--param3".to_string(),
                "value3".to_string(),
                "param4".to_string(),
                "param5".to_string()
            ],]
        );
        let p_make_params = p_make_params_arc.lock().unwrap();
        assert_eq!(
            *p_make_params,
            vec![vec![
                "command".to_string(),
                "--param1".to_string(),
                "value1".to_string(),
                "--param2".to_string(),
                "value2".to_string(),
                "subcommand".to_string(),
                "--param3".to_string(),
                "value3".to_string(),
                "param4".to_string(),
                "param5".to_string(),
            ]]
        );
        let mut process_params = process_params_arc.lock().unwrap();
        let command = process_params.remove(0);
        let transact_params_arc = Arc::new(Mutex::new(vec![]));
        let mut context = CommandContextMock::new()
            .transact_params(&transact_params_arc)
            .transact_result(Err(Other("not really an error".to_string())));
        let stdout_arc = context.stdout_arc();
        let stderr_arc = context.stderr_arc();

        let result = command.execute(&mut context);

        assert_eq!(
            result,
            Err(Transmission("Other(\"not really an error\")".to_string()))
        );
        let transact_params = transact_params_arc.lock().unwrap();
        assert_eq!(*transact_params, vec![(UiShutdownRequest {}.tmb(1), 1000)]);
        assert_eq!(
            stdout_arc.lock().unwrap().get_string(),
            "MockCommand output".to_string()
        );
        assert_eq!(
            stderr_arc.lock().unwrap().get_string(),
            "MockCommand error".to_string()
        );
    }

    #[derive(Debug)]
    struct FakeCommand {
        output: String,
    }

    impl commands_common::Command for FakeCommand {
        fn execute(&self, _context: &mut dyn CommandContext) -> Result<(), CommandError> {
            intentionally_blank!()
        }
    }

    impl FakeCommand {
        pub fn new(output: &str) -> Self {
            Self {
                output: output.to_string(),
            }
        }
    }

    #[test]
    fn interactive_mode_works_when_everything_is_copacetic() {
        let make_params_arc = Arc::new(Mutex::new(vec![]));
        let command_factory = CommandFactoryMock::new()
            .make_params(&make_params_arc)
            .make_result(Ok(Box::new(FakeCommand::new("setup command"))))
            .make_result(Ok(Box::new(FakeCommand::new("start command"))));
        let terminal_mock = TerminalPassiveMock::new()
            .read_line_result(TerminalEvent::CommandLine("setup".to_string()))
            .read_line_result(TerminalEvent::CommandLine("start".to_string()))
            .read_line_result(TerminalEvent::CommandLine("exit".to_string()));
        let interface = InterfaceMock::new()
            .make_result(Ok(TerminalReal::new(Box::new(InterfaceRawMock::new()))));
        let processor = CommandProcessorMock::new()
            .process_result(Ok(()))
            .process_result(Ok(()))
            .insert_terminal_interface(TerminalWrapper::new(Box::new(terminal_mock)));
        let processor_factory =
            CommandProcessorFactoryMock::new().make_result(Ok(Box::new(processor)));
        let mut subject = Main {
            command_factory: Box::new(command_factory),
            processor_factory: Box::new(processor_factory),
            terminal_interface_factory: Box::new(interface),
        };
        let mut stream_holder = FakeStreamHolder::new();

        let result = subject.go(
            &mut stream_holder.streams(),
            &[
                "command".to_string(),
                "--param1".to_string(),
                "value1".to_string(),
            ],
        );

        assert_eq!(result, 0);
        let make_params = make_params_arc.lock().unwrap();
        assert_eq!(
            *make_params,
            vec![vec!["setup".to_string()], vec!["start".to_string()]]
        );
    }

    #[test]
    fn interactive_mode_works_for_stdin_read_error() {
        let command_factory = CommandFactoryMock::new();
        let close_params_arc = Arc::new(Mutex::new(vec![]));
        let processor = CommandProcessorMock::new()
            .close_params(&close_params_arc)
            .insert_terminal_interface(TerminalWrapper::new(Box::new(
                TerminalPassiveMock::new()
                    .read_line_result(TerminalEvent::Error("ConnectionRefused".to_string())),
            )));
        let processor_factory =
            CommandProcessorFactoryMock::new().make_result(Ok(Box::new(processor)));
        let interface = InterfaceMock::new()
            .make_result(Ok(TerminalReal::new(Box::new(InterfaceRawMock::new()))));
        let mut subject = Main {
            command_factory: Box::new(command_factory),
            processor_factory: Box::new(processor_factory),
            terminal_interface_factory: Box::new(interface),
        };
        let mut stream_holder = FakeStreamHolder::new();

        let result = subject.go(&mut stream_holder.streams(), &["command".to_string()]);

        assert_eq!(result, 1);
        assert_eq!(
            stream_holder.stderr.get_string(),
            "ConnectionRefused\n".to_string()
        );
        let close_params = close_params_arc.lock().unwrap();
        assert_eq!(close_params.len(), 1);
    }

    #[test]
    fn interactive_mode_works_for_unrecognized_command() {
        let make_params_arc = Arc::new(Mutex::new(vec![]));
        let command_factory = CommandFactoryMock::new()
            .make_params(&make_params_arc)
            .make_result(Err(CommandFactoryError::UnrecognizedSubcommand(
                "Booga!".to_string(),
            )));
        let interface = InterfaceMock::new()
            .make_result(Ok(TerminalReal::new(Box::new(InterfaceRawMock::new()))));
        let processor =
            CommandProcessorMock::new().insert_terminal_interface(TerminalWrapper::new(Box::new(
                TerminalPassiveMock::new()
                    .read_line_result(TerminalEvent::CommandLine("error command\n".to_string()))
                    .read_line_result(TerminalEvent::CommandLine("exit\n".to_string())),
            )));
        let processor_factory =
            CommandProcessorFactoryMock::new().make_result(Ok(Box::new(processor)));
        let mut subject = Main {
            command_factory: Box::new(command_factory),
            processor_factory: Box::new(processor_factory),
            terminal_interface_factory: Box::new(interface),
        };
        let mut stream_holder = FakeStreamHolder::new();

        let result = subject.go(&mut stream_holder.streams(), &["command".to_string()]);

        assert_eq!(result, 0);
        let make_params = make_params_arc.lock().unwrap();
        assert_eq!(
            *make_params,
            vec![vec!["error".to_string(), "command".to_string()]]
        );
        assert_eq!(
            stream_holder.stderr.get_string(),
            "Unrecognized command: 'Booga!'\n".to_string()
        );
    }

    #[test]
    fn interactive_mode_works_for_command_with_bad_syntax() {
        let make_params_arc = Arc::new(Mutex::new(vec![]));
        let command_factory = CommandFactoryMock::new()
            .make_params(&make_params_arc)
            .make_result(Err(CommandFactoryError::CommandSyntax(
                "Booga!".to_string(),
            )));
        let interface = InterfaceMock::new()
            .make_result(Ok(TerminalReal::new(Box::new(InterfaceRawMock::new()))));
        let processor =
            CommandProcessorMock::new().insert_terminal_interface(TerminalWrapper::new(Box::new(
                TerminalPassiveMock::new()
                    .read_line_result(TerminalEvent::CommandLine("error command\n".to_string()))
                    .read_line_result(TerminalEvent::CommandLine("exit\n".to_string())),
            )));
        let processor_factory =
            CommandProcessorFactoryMock::new().make_result(Ok(Box::new(processor)));
        let mut subject = Main {
            command_factory: Box::new(command_factory),
            processor_factory: Box::new(processor_factory),
            terminal_interface_factory: Box::new(interface),
        };
        let mut stream_holder = FakeStreamHolder::new();

        let result = subject.go(&mut stream_holder.streams(), &["command".to_string()]);

        assert_eq!(result, 0);
        let make_params = make_params_arc.lock().unwrap();
        assert_eq!(
            *make_params,
            vec![vec!["error".to_string(), "command".to_string()]]
        );
        assert_eq!(stream_holder.stderr.get_string(), "Booga!\n".to_string());
    }

    #[test]
    fn clone_of_synchronizer_is_shared_along_and_passed_on_properly() {
        let make_params_arc = Arc::new(Mutex::new(vec![]));
        let command_factory = CommandFactoryMock::new()
            .make_params(&make_params_arc)
            .make_result(Ok(Box::new(FakeCommand::new("setup command"))));
        let interface = InterfaceMock::new()
            .make_result(Ok(TerminalReal::new(Box::new(InterfaceRawMock::new()))));
        let terminal_interface_reference_for_inner = TerminalWrapper::new(Box::new(
            TerminalPassiveMock::new()
                .read_line_result(TerminalEvent::CommandLine("setup\n".to_string()))
                .read_line_result(TerminalEvent::CommandLine("exit\n".to_string())),
        ));
        let reference_for_counting = Arc::new(Mutex::new(0));
        let processor = CommandProcessorMock::new()
            .insert_terminal_interface(terminal_interface_reference_for_inner.clone())
            .insert_terminal_wrapper_shared_counter(reference_for_counting.clone())
            .process_result(Ok(()));

        assert_eq!(*reference_for_counting.lock().unwrap(), 0);

        let processor_factory =
            CommandProcessorFactoryMock::new().make_result(Ok(Box::new(processor)));
        let mut subject = Main {
            command_factory: Box::new(command_factory),
            processor_factory: Box::new(processor_factory),
            terminal_interface_factory: Box::new(interface),
        };
        let mut stream_holder = FakeStreamHolder::new();

        let result = subject.go(
            &mut stream_holder.streams(),
            &[
                "command".to_string(),
                "--param1".to_string(),
                "value1".to_string(),
            ],
        );

        //cloned once for each command, so twice in total
        assert_eq!(*reference_for_counting.lock().unwrap(), 2);

        assert_eq!(result, 0);
        let make_params = make_params_arc.lock().unwrap();
        assert_eq!(*make_params, vec![vec!["setup".to_string()]]);
    }

    #[test]
    fn go_works_when_error_turns_up_in_interface_factory() {
        let c_make_params_arc = Arc::new(Mutex::new(vec![]));
        let command_factory = CommandFactoryMock::new().make_params(&c_make_params_arc);
        let interface = InterfaceMock::new().make_result(Err("Invalid handle".to_string()));
        let mut subject = Main {
            command_factory: Box::new(command_factory),
            processor_factory: Box::new(CommandProcessorFactoryMock::new()),
            terminal_interface_factory: Box::new(interface),
        };
        let mut stream_holder = FakeStreamHolder::new();

        let result = subject.go(
            &mut stream_holder.streams(),
            &["command".to_string(), "subcommand".to_string()],
        );

        assert_eq!(result, 1);
        let c_make_params = c_make_params_arc.lock().unwrap();
        assert!(c_make_params.is_empty());
        assert_eq!(
            stream_holder.stderr.get_string(),
            "Terminal interface: Invalid handle\n".to_string()
        );
    }

    #[test]
    fn go_works_when_command_is_unrecognized() {
        let c_make_params_arc = Arc::new(Mutex::new(vec![]));
        let command_factory = CommandFactoryMock::new()
            .make_params(&c_make_params_arc)
            .make_result(Err(UnrecognizedSubcommand("booga".to_string())));
        let close_params_arc = Arc::new(Mutex::new(vec![]));
        let interface = InterfaceMock::new()
            .make_result(Ok(TerminalReal::new(Box::new(InterfaceRawMock::new()))));
        let processor = CommandProcessorMock::new().close_params(&close_params_arc);
        let processor_factory =
            CommandProcessorFactoryMock::new().make_result(Ok(Box::new(processor)));
        let mut subject = Main {
            command_factory: Box::new(command_factory),
            processor_factory: Box::new(processor_factory),
            terminal_interface_factory: Box::new(interface),
        };
        let mut stream_holder = FakeStreamHolder::new();

        let result = subject.go(
            &mut stream_holder.streams(),
            &["command".to_string(), "subcommand".to_string()],
        );

        assert_eq!(result, 1);
        let c_make_params = c_make_params_arc.lock().unwrap();
        assert_eq!(*c_make_params, vec![vec!["subcommand".to_string()],]);
        assert_eq!(
            stream_holder.stderr.get_string(),
            "Unrecognized command: 'booga'\n".to_string()
        );
        let close_params = close_params_arc.lock().unwrap();
        assert_eq!(close_params.len(), 1);
    }

    #[test]
    fn go_works_when_command_has_bad_syntax() {
        let c_make_params_arc = Arc::new(Mutex::new(vec![]));
        let command_factory = CommandFactoryMock::new()
            .make_params(&c_make_params_arc)
            .make_result(Err(CommandSyntax("booga".to_string())));
        let interface = InterfaceMock::new()
            .make_result(Ok(TerminalReal::new(Box::new(InterfaceRawMock::new()))));
        let processor = CommandProcessorMock::new();
        let processor_factory =
            CommandProcessorFactoryMock::new().make_result(Ok(Box::new(processor)));
        let mut subject = Main {
            command_factory: Box::new(command_factory),
            processor_factory: Box::new(processor_factory),
            terminal_interface_factory: Box::new(interface),
        };
        let mut stream_holder = FakeStreamHolder::new();

        let result = subject.go(
            &mut stream_holder.streams(),
            &["command".to_string(), "subcommand".to_string()],
        );

        assert_eq!(result, 1);
        let c_make_params = c_make_params_arc.lock().unwrap();
        assert_eq!(*c_make_params, vec![vec!["subcommand".to_string()],]);
        assert_eq!(stream_holder.stdout.get_string(), "".to_string());
        assert_eq!(stream_holder.stderr.get_string(), "booga\n".to_string());
    }

    #[test]
    fn go_works_when_command_execution_fails() {
        let command = MockCommand::new(UiShutdownRequest {}.tmb(1)).execute_result(Ok(())); // irrelevant
        let command_factory = CommandFactoryMock::new().make_result(Ok(Box::new(command)));
        let process_params_arc = Arc::new(Mutex::new(vec![]));
        let interface = InterfaceMock::new()
            .make_result(Ok(TerminalReal::new(Box::new(InterfaceRawMock::new()))));
        let processor = CommandProcessorMock::new()
            .process_params(&process_params_arc)
            .process_result(Err(Transmission("Booga!".to_string())));
        let processor_factory =
            CommandProcessorFactoryMock::new().make_result(Ok(Box::new(processor)));
        let mut subject = Main {
            command_factory: Box::new(command_factory),
            processor_factory: Box::new(processor_factory),
            terminal_interface_factory: Box::new(interface),
        };
        let mut stream_holder = FakeStreamHolder::new();

        let result = subject.go(
            &mut stream_holder.streams(),
            &["command".to_string(), "subcommand".to_string()],
        );

        assert_eq!(result, 1);
        assert_eq!(stream_holder.stdout.get_string(), "".to_string());
        assert_eq!(
            stream_holder.stderr.get_string(),
            "Transmission problem: Booga!\n".to_string()
        );
    }

    #[test]
    fn go_works_when_daemon_is_not_running() {
        let interface = InterfaceMock::new()
            .make_result(Ok(TerminalReal::new(Box::new(InterfaceRawMock::new()))));
        let processor_factory = CommandProcessorFactoryMock::new()
            .make_result(Err(CommandError::ConnectionProblem("booga".to_string())));
        let mut subject = Main {
            command_factory: Box::new(CommandFactoryMock::new()),
            processor_factory: Box::new(processor_factory),
            terminal_interface_factory: Box::new(interface),
        };
        let mut stream_holder = FakeStreamHolder::new();

        let result = subject.go(
            &mut stream_holder.streams(),
            &["command".to_string(), "subcommand".to_string()],
        );

        assert_eq!(result, 1);
        assert_eq!(stream_holder.stdout.get_string(), "".to_string());
        assert_eq!(
            stream_holder.stderr.get_string(),
            "Can't connect to Daemon or Node (ConnectionProblem(\"booga\")). \
             Probably this means the Daemon isn't running.\n"
                .to_string()
        );
    }
}

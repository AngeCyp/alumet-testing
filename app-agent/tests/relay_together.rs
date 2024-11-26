//! Integration tests for the relay mode, client and server together.

use std::{
    process::{self, ExitStatus, Stdio},
    time::Duration,
};

use anyhow::{anyhow, Context};
use common::run::{command_cargo_build, command_cargo_run, ChildGuard};

mod common;

/// Check that the client can send measurements to the server,
/// which will write them to a CSV file.
///
/// Note: we use a limited set of plugins so that it works in the CI environment.
#[test]
fn client_to_server_to_csv() {
    // These tests are in the same test function because they must NOT run concurrently (same port).

    // works in CI
    client_to_server_to_csv_on_address("ipv4", Some(("localhost", "50051"))).unwrap();

    // doesn't work in CI
    if std::env::var_os("NO_IPV6").is_some() {
        println!("IPv6 test disabled by environment variable.");
    } else {
        client_to_server_to_csv_on_address("ipv6", Some(("::1", "50051"))).unwrap();
        client_to_server_to_csv_on_address("default", None).unwrap();
    }
}

fn client_to_server_to_csv_on_address(
    tag: &str,
    addr_and_port: Option<(&'static str, &'static str)>,
) -> anyhow::Result<()> {
    let tmp_dir = std::env::temp_dir().join(format!("{}-client_to_server_to_csv-{tag}", env!("CARGO_CRATE_NAME")));
    match std::fs::remove_dir_all(&tmp_dir) {
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(anyhow!("failed to remove dir {tmp_dir:?}: {e}")),
    }?;
    std::fs::create_dir(&tmp_dir).with_context(|| format!("failed to create dir {tmp_dir:?}"))?;

    let server_config = tmp_dir.join("server.toml");
    let client_config = tmp_dir.join("client.toml");
    let server_output = tmp_dir.join("output.csv");
    assert!(
        matches!(std::fs::exists(&server_config), Ok(false)),
        "server config should not exist"
    );
    assert!(
        matches!(std::fs::exists(&client_config), Ok(false)),
        "client config should not exist"
    );
    assert!(
        matches!(std::fs::exists(&server_output), Ok(false)),
        "server output file should not exist"
    );

    let server_config_str = server_config.to_str().unwrap().to_owned();
    let client_config_str = client_config.to_str().unwrap().to_owned();
    let server_output_str = server_output.to_str().unwrap().to_owned();

    // Build before run to avoid delays too big.
    command_cargo_build("alumet-relay-server", &["relay_server"])
        .spawn()?
        .wait()
        .context("cargo build should run to completion")?;
    command_cargo_build("alumet-relay-client", &["relay_client"])
        .spawn()?
        .wait()
        .context("cargo build should run to completion")?;

    // Spawn the server
    let server_csv_output_conf = format!("plugins.csv.output_path='''{server_output_str}'''");
    let mut server_args = Vec::from_iter([
        "--config",
        &server_config_str,
        // only enable some plugins
        "--plugins=relay-server,csv",
        // ensure that the CSV plugin flushes the buffer to the file ASAP
        "--config-override",
        "plugins.csv.force_flush=true",
        // set the CSV output to the file we want
        "--config-override",
        &server_csv_output_conf,
    ]);
    if let Some((server_addr, server_port)) = addr_and_port {
        server_args.extend_from_slice(&["--address", server_addr, "--port", server_port]);
    }
    let server_process: process::Child = command_cargo_run("alumet-relay-server", &["relay_server"], &server_args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("server process should spawn")?;
    let mut server_process = ChildGuard::new(server_process);
    println!("spawned server process {}", server_process.id());

    // Wait for the server to start
    let mut loop_limit = 50;
    while !std::fs::exists(&server_config).context("could not check existence of config")? {
        if loop_limit == 0 {
            let _ = server_process.kill();
            panic!("The server config is not generated! Config path: {server_config_str}");
        }
        std::thread::sleep(Duration::from_millis(50));
        loop_limit -= 1;
    }
    std::thread::sleep(Duration::from_millis(250));

    // Start the client
    let mut client_args: Vec<String> = Vec::from_iter([
        // use a different config than the server
        "--config",
        &client_config_str,
        // only enable some plugins
        "--plugins=relay-client,procfs",
        // override the config to lower the poll_interval (so that the test is faster)
        "--config-override",
        "plugins.procfs.kernel.poll_interval='50ms'",
        "--config-override",
        "plugins.procfs.memory.poll_interval='50ms'",
        "--config-override",
        "plugins.procfs.processes.enabled=false",
        // don't buffer the relay output (because we want to check the final output after a short delay)
        "--config-override",
        "plugins.relay-client.buffer_max_length=0",
    ])
    .into_iter()
    .map(String::from)
    .collect();

    if let Some((server_addr, port)) = addr_and_port {
        let addr_in_uri = if server_addr.contains(':') {
            format!("[{server_addr}]")
        } else {
            server_addr.to_owned()
        };
        client_args.extend_from_slice(&[
            // specify an URI that works in the CI
            "--relay-server".into(),
            format!("{addr_in_uri}:{port}"),
        ]);
    }
    let client_args: Vec<&str> = client_args.iter().map(|s| s.as_str()).collect();

    let client_process = command_cargo_run("alumet-relay-client", &["relay_client"], &client_args)
        // .stdout(Stdio::piped())
        // .stderr(Stdio::piped())
        .env("RUST_LOG", "debug")
        .spawn()?;
    let mut client_process = ChildGuard::new(client_process);
    println!("spawned client process {}", client_process.id());

    // Wait a little bit
    let delta = Duration::from_millis(1000);
    std::thread::sleep(delta);

    // Check that the processes still run
    assert!(
        matches!(client_process.try_wait(), Ok(None)),
        "the client should still run after a while"
    );
    assert!(
        matches!(server_process.try_wait(), Ok(None)),
        "the server should still run after a while"
    );

    // Check that we've obtained some measurements
    let output_content_before_stop = std::fs::read_to_string(&server_output)?;
    // assert!(
    //     !output_content_before_stop.is_empty(),
    //     "some measurements should have been written after {delta:?}"
    // );

    // Stop the client
    kill_gracefully(&mut client_process)?;

    // Wait for the client to stop (TODO: a timeout would be nice, but it's no so simple to have)
    let client_status = client_process.take().wait()?;
    assert!(
        stopped_gracefully(client_status),
        "the client should exit in a controlled way, but had status {client_status}"
    );

    // Check that we still have measurements
    let output_content_after_stop = std::fs::read_to_string(&server_output)?;
    assert!(
        !output_content_after_stop.is_empty(),
        "some measurements should have been written after the client shutdown"
    );

    // Stop the server
    kill_gracefully(&mut server_process)?;

    // Wait for the server to be stopped.
    let server_output = server_process.take().wait_with_output()?;
    let server_status = server_output.status;
    println!(
        "vvvvvvvvvvvv server output below vvvvvvvvvvvv\n{}\n------\n{}\n------\n",
        String::from_utf8(server_output.stdout).unwrap(),
        String::from_utf8(server_output.stderr).unwrap()
    );
    assert!(
        stopped_gracefully(server_status),
        "the server should exit in a controlled way, but had status {server_status}"
    );
    Ok(())
}

fn stopped_gracefully(status: ExitStatus) -> bool {
    use std::os::unix::process::ExitStatusExt;
    status.success() || status.signal().is_some()
}

fn kill_gracefully(child: &mut process::Child) -> anyhow::Result<()> {
    let res = unsafe { libc::kill(child.id() as i32, libc::SIGTERM) };
    if res == 0 {
        Ok(())
    } else {
        Err(anyhow!("failed to kill process {}", child.id()))
    }
}

use crate::settings::remote_hosts::RemoteHost;

/// `[-p PORT, -i IDENTITY?, OPTIONS..., HOST]` — the host-targeting tail of an ssh invocation.
pub(crate) fn target_args(host: &RemoteHost) -> Vec<String> {
    let mut args = Vec::with_capacity(4 + host.ssh_options.len());
    args.push("-p".into());
    args.push(host.port.to_string());
    if let Some(id) = host.identity_file_arg() {
        args.push("-i".into());
        args.push(id.to_owned());
    }
    args.extend(host.ssh_options.iter().cloned());
    args.push(host.host.clone());
    args
}

/// Same as [`target_args`] but each user-controllable value is passed through `shell_words::quote`.
/// Use when the result will be embedded in a shell command string (not passed to `Command::args`).
pub(crate) fn target_args_shell_quoted(host: &RemoteHost) -> Vec<String> {
    let mut args = Vec::with_capacity(4 + host.ssh_options.len());
    args.push("-p".into());
    args.push(host.port.to_string());
    if let Some(id) = host.identity_file_arg() {
        args.push("-i".into());
        args.push(shell_words::quote(id).into_owned());
    }
    for opt in &host.ssh_options {
        args.push(shell_words::quote(opt).into_owned());
    }
    args.push(shell_words::quote(&host.host).into_owned());
    args
}

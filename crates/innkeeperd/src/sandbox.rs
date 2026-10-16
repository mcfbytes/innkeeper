//! Landlock, seccomp and the root check, applied before the runtime starts so workers inherit
//! them (docs/server/hardening.md).

use std::io::{self, Write};
use std::path::Path;
use std::process::ExitCode;

use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum SandboxError {
    #[error("refusing to run as root; pass --allow-root only for a local DOSBox setup")]
    Root,
    #[error("cannot set no_new_privs: errno {0}")]
    NoNewPrivs(i32),
    #[error("landlock: {0}")]
    Landlock(String),
    #[error("seccomp: {0}")]
    Seccomp(String),
}

/// What confinement needs from a process that has already bound its sockets.
pub(crate) struct ConfineRequest<'a> {
    pub(crate) euid: u32,
    pub(crate) allow_root: bool,
    pub(crate) log_only: bool,
    pub(crate) data_dir: Option<&'a Path>,
    pub(crate) capture_dir: Option<&'a Path>,
    pub(crate) ports: &'a [u16],
}

pub(crate) fn refuse_root(euid: u32, allow_root: bool) -> Result<(), SandboxError> {
    if euid == 0 && !allow_root {
        Err(SandboxError::Root)
    } else {
        Ok(())
    }
}

pub(crate) fn current_euid() -> u32 {
    #[cfg(target_os = "linux")]
    {
        // libc marks geteuid unsafe, and this crate forbids unsafe. Uid is `real effective`.
        linux_euid()
    }
    #[cfg(not(target_os = "linux"))]
    {
        1
    }
}

/// Effective uid from `/proc/self/status`. Unreadable proc is treated as root.
#[cfg(target_os = "linux")]
fn linux_euid() -> u32 {
    let Ok(text) = std::fs::read_to_string("/proc/self/status") else {
        return 0;
    };
    for line in text.lines() {
        let Some(rest) = line.strip_prefix("Uid:") else {
            continue;
        };
        let mut fields = rest.split_whitespace();
        let _real = fields.next();
        if let Some(euid) = fields.next().and_then(|field| field.parse().ok()) {
            return euid;
        }
    }
    0
}

/// Map `localhost` so glibc's files resolver is already loaded. An external name is not resolved.
pub(crate) fn preload_resolver() {
    use std::net::ToSocketAddrs;
    let _ = ("localhost", 1u16).to_socket_addrs();
}

pub(crate) fn confine(request: &ConfineRequest<'_>) -> Result<(), SandboxError> {
    #[cfg(target_os = "linux")]
    {
        linux::apply(request)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = request;
        tracing::warn!("sandbox: --sandbox is unsupported on this operating system");
        Ok(())
    }
}

/// Hidden self-check: print whether PATH could be opened and exit.
pub(crate) fn run_probe(path: &Path) -> ExitCode {
    let (line, code) = match std::fs::File::open(path) {
        Ok(_file) => ("sandbox-probe: opened", 1),
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
            ("sandbox-probe: denied", 0)
        }
        Err(error) => {
            eprintln!("sandbox-probe: {error}");
            return ExitCode::from(2);
        }
    };
    println!("{line}");
    let _ = io::stdout().flush();
    ExitCode::from(code)
}

pub(crate) fn summary_line(abi: &str, fs: &str, net: &str, seccomp: &str) -> String {
    format!("sandbox: landlock abi={abi} fs={fs} net={net}, seccomp={seccomp}, no_new_privs")
}

#[cfg(target_os = "linux")]
mod filter;

#[cfg(target_os = "linux")]
mod linux {
    use std::path::{Path, PathBuf};

    use landlock::{
        Access, AccessFs, AccessNet, CompatLevel, Compatible, LandlockStatus, NetPort, PathBeneath,
        PathFd, PathFdError, RestrictSelfAttr, RestrictionStatus, Ruleset, RulesetAttr,
        RulesetCreated, RulesetCreatedAttr, RulesetStatus, ABI,
    };
    use seccompiler::{apply_filter, BpfProgram, SeccompAction, TargetArch};

    use super::filter::build_filter;
    use super::{summary_line, ConfineRequest, SandboxError};

    pub(super) fn apply(request: &ConfineRequest<'_>) -> Result<(), SandboxError> {
        super::refuse_root(request.euid, request.allow_root)?;
        prctl::set_no_new_privileges(true).map_err(SandboxError::NoNewPrivs)?;
        warn_if_capture_is_outside(request.data_dir, request.capture_dir);
        let (abi, fs, net) = apply_landlock(request.data_dir, request.ports)?;
        let seccomp = apply_seccomp(request.log_only)?;
        tracing::info!("{}", summary_line(&abi, fs, net, seccomp));
        Ok(())
    }

    fn warn_if_capture_is_outside(data_dir: Option<&Path>, capture_dir: Option<&Path>) {
        let Some(capture) = capture_dir else {
            return;
        };
        let inside = data_dir.is_some_and(|root| path_is_inside(capture, root));
        if !inside {
            tracing::warn!(
                capture = %capture.display(),
                "sandbox: the capture directory is outside --data-dir, so capture writes will be denied"
            );
        }
    }

    fn path_is_inside(path: &Path, root: &Path) -> bool {
        let Ok(path) = path.canonicalize() else {
            return false;
        };
        let Ok(root) = root.canonicalize() else {
            return false;
        };
        path.starts_with(root)
    }

    /// ReadFile and ReadDir only. Execute stays handled, so it is denied.
    pub(super) fn read_only_access() -> landlock::BitFlags<AccessFs> {
        AccessFs::ReadFile | AccessFs::ReadDir
    }

    /// The data directory. Execute, devices and ioctl-dev stay denied.
    pub(super) fn read_write_access() -> landlock::BitFlags<AccessFs> {
        read_only_access()
            | AccessFs::WriteFile
            | AccessFs::RemoveDir
            | AccessFs::RemoveFile
            | AccessFs::MakeDir
            | AccessFs::MakeReg
            | AccessFs::Refer
            | AccessFs::Truncate
    }

    fn apply_landlock(
        data_dir: Option<&Path>,
        ports: &[u16],
    ) -> Result<(String, &'static str, &'static str), SandboxError> {
        let built = build_ruleset(data_dir, ports)?;
        if !built.missing.is_empty() {
            tracing::warn!(
                paths = %built.missing.join(", "),
                "sandbox: skipped missing read-only paths"
            );
        }
        // ABI v8 would cover sibling threads. Best effort drops that flag here; children inherit.
        let status = built
            .ruleset
            .all_threads(true)
            .map_err(land_err)?
            .restrict_self()
            .map_err(land_err)?;
        Ok(describe(&status))
    }

    struct Built {
        ruleset: RulesetCreated,
        missing: Vec<String>,
    }

    #[cfg(test)]
    pub(super) fn ruleset_builds(
        data_dir: Option<&Path>,
        ports: &[u16],
    ) -> Result<(), SandboxError> {
        build_ruleset(data_dir, ports).map(|_| ())
    }

    fn build_ruleset(data_dir: Option<&Path>, ports: &[u16]) -> Result<Built, SandboxError> {
        // ConnectTcp is not handled, so outbound TLS (Key Vault, IMDS) stays allowed.
        let mut ruleset = Ruleset::default()
            .set_compatibility(CompatLevel::BestEffort)
            .handle_access(AccessFs::from_all(ABI::V5))
            .map_err(land_err)?
            .handle_access(AccessNet::BindTcp)
            .map_err(land_err)?
            .create()
            .map_err(land_err)?;
        if let Some(dir) = data_dir {
            ruleset = grant(ruleset, dir, read_write_access())?;
        }
        let mut missing = Vec::new();
        for path in read_only_paths() {
            ruleset = grant_optional(ruleset, &path, &mut missing)?;
        }
        for port in unique_ports(ports) {
            ruleset = ruleset
                .add_rule(NetPort::new(port, AccessNet::BindTcp))
                .map_err(land_err)?;
        }
        Ok(Built { ruleset, missing })
    }

    fn read_only_paths() -> Vec<PathBuf> {
        let mut paths = [
            "/etc/ssl",
            "/etc/ssl/certs",
            "/etc/ssl/certs/ca-certificates.crt",
            "/usr/share/ca-certificates",
            "/etc/resolv.conf",
            "/etc/hosts",
            "/etc/nsswitch.conf",
            "/etc/gai.conf",
            "/etc/host.conf",
            "/etc/localtime",
            "/usr/share/zoneinfo",
            "/proc/self",
            "/proc/cpuinfo",
            "/proc/stat",
            "/sys/devices/system/cpu",
            "/sys/fs/cgroup",
            "/dev/urandom",
        ]
        .map(PathBuf::from)
        .to_vec();
        paths.push(PathBuf::from(format!("/proc/{}", std::process::id())));
        paths
    }

    fn grant(
        ruleset: RulesetCreated,
        path: &Path,
        access: landlock::BitFlags<AccessFs>,
    ) -> Result<RulesetCreated, SandboxError> {
        let fd = PathFd::new(path).map_err(|error| SandboxError::Landlock(error.to_string()))?;
        ruleset
            .add_rule(PathBeneath::new(fd, access))
            .map_err(land_err)
    }

    fn grant_optional(
        ruleset: RulesetCreated,
        path: &Path,
        missing: &mut Vec<String>,
    ) -> Result<RulesetCreated, SandboxError> {
        if !path.exists() {
            missing.push(path.display().to_string());
            return Ok(ruleset);
        }
        let ruleset = match PathFd::new(path) {
            Ok(fd) => ruleset
                .add_rule(PathBeneath::new(fd, read_only_access()))
                .map_err(land_err)?,
            Err(PathFdError::OpenCall { source, .. })
                if source.kind() == std::io::ErrorKind::NotFound =>
            {
                missing.push(path.display().to_string());
                return Ok(ruleset);
            }
            Err(error) => return Err(SandboxError::Landlock(error.to_string())),
        };
        let Ok(target) = path.canonicalize() else {
            return Ok(ruleset);
        };
        if target == path {
            return Ok(ruleset);
        }
        grant(ruleset, &target, read_only_access())
    }

    fn unique_ports(ports: &[u16]) -> Vec<u16> {
        let mut out = Vec::new();
        for port in ports {
            if *port != 0 && !out.contains(port) {
                out.push(*port);
            }
        }
        out
    }

    fn describe(status: &RestrictionStatus) -> (String, &'static str, &'static str) {
        let enforced = status.ruleset != RulesetStatus::NotEnforced;
        let described = match status.landlock {
            LandlockStatus::Available { effective_abi, .. } if enforced => {
                let net = if effective_abi >= ABI::V4 {
                    "bind-restricted"
                } else {
                    "unrestricted"
                };
                (effective_abi.to_string(), "restricted", net)
            }
            _ => ("unsupported".to_string(), "open", "unrestricted"),
        };
        if !enforced {
            tracing::warn!(
                "sandbox: this kernel has no Landlock; filesystem and TCP bind stay unrestricted"
            );
        }
        described
    }

    fn land_err(error: impl std::fmt::Display) -> SandboxError {
        SandboxError::Landlock(error.to_string())
    }

    fn apply_seccomp(log_only: bool) -> Result<&'static str, SandboxError> {
        let Some(arch) = target_arch() else {
            tracing::warn!("sandbox: seccomp is not implemented for this architecture");
            return Ok("unsupported");
        };
        let mismatch = if log_only {
            SeccompAction::Log
        } else {
            SeccompAction::KillProcess
        };
        let filter = build_filter(arch, mismatch).map_err(seccomp_err)?;
        let program: BpfProgram = filter.try_into().map_err(seccomp_err)?;
        apply_filter(&program).map_err(seccomp_err)?;
        Ok(if log_only { "log" } else { "kill" })
    }

    fn seccomp_err(error: impl std::fmt::Display) -> SandboxError {
        SandboxError::Seccomp(error.to_string())
    }

    fn target_arch() -> Option<TargetArch> {
        match std::env::consts::ARCH {
            "x86_64" => Some(TargetArch::x86_64),
            "aarch64" => Some(TargetArch::aarch64),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_is_refused_unless_allow_root_is_set() {
        let refused = refuse_root(0, false).unwrap_err();
        assert!(refused.to_string().contains("refusing to run as root"));
        assert!(refuse_root(0, true).is_ok());
        assert!(refuse_root(1000, false).is_ok());
    }

    #[test]
    fn the_summary_line_names_each_control() {
        assert_eq!(
            summary_line("7", "restricted", "bind-restricted", "kill"),
            "sandbox: landlock abi=7 fs=restricted net=bind-restricted, seccomp=kill, no_new_privs"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn read_only_rules_grant_no_execute_and_the_data_dir_is_writable() {
        let read = linux::read_only_access();
        assert!(read.contains(landlock::AccessFs::ReadFile));
        assert!(!read.contains(landlock::AccessFs::Execute));
        assert!(!read.contains(landlock::AccessFs::WriteFile));
        let write = linux::read_write_access();
        assert!(write.contains(landlock::AccessFs::WriteFile));
        assert!(write.contains(landlock::AccessFs::RemoveFile));
        assert!(write.contains(landlock::AccessFs::Truncate));
        assert!(write.contains(landlock::AccessFs::MakeReg));
        assert!(!write.contains(landlock::AccessFs::Execute));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_ruleset_can_be_built_without_restricting_this_process() {
        let dir = std::env::temp_dir().join(format!("inn-sandbox-rules-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let built = linux::ruleset_builds(Some(&dir), &[9]);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(built.is_ok(), "{built:?}");
    }

    #[cfg(all(
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64")
    ))]
    #[test]
    fn the_allowlist_omits_process_control_and_the_filter_builds() {
        let allowed = super::filter::syscall_numbers();
        for number in [
            libc::SYS_execve,
            libc::SYS_execveat,
            libc::SYS_ptrace,
            libc::SYS_mount,
            libc::SYS_unshare,
            libc::SYS_setns,
            libc::SYS_bpf,
            libc::SYS_perf_event_open,
            libc::SYS_keyctl,
            libc::SYS_init_module,
            libc::SYS_finit_module,
            libc::SYS_delete_module,
            libc::SYS_kexec_load,
            libc::SYS_kexec_file_load,
            libc::SYS_reboot,
            libc::SYS_swapon,
            libc::SYS_swapoff,
            libc::SYS_personality,
        ] {
            assert!(!allowed.contains(&number), "syscall {number} is allowed");
        }
        assert!(allowed.contains(&libc::SYS_futex));
        assert!(!allowed.contains(&libc::SYS_execve));
        let arch = match std::env::consts::ARCH {
            "x86_64" => seccompiler::TargetArch::x86_64,
            "aarch64" => seccompiler::TargetArch::aarch64,
            other => panic!("unexpected arch {other}"),
        };
        let filter = super::filter::build_filter(arch, seccompiler::SeccompAction::KillProcess);
        let program: seccompiler::BpfProgram = filter.unwrap().try_into().unwrap();
        assert!(!program.is_empty());
        assert!(
            program.len() < 4096,
            "filter is {} instructions",
            program.len()
        );
        eprintln!("allowlist {}", allowed.len());
    }
}

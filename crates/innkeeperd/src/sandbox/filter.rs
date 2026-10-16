//! Seccomp allowlist for the runtime and sqlite (docs/server/hardening.md).

use seccompiler::{
    SeccompAction, SeccompCmpArgLen, SeccompCmpOp, SeccompCondition, SeccompFilter, SeccompRule,
    TargetArch,
};

pub(super) fn build_filter(
    arch: TargetArch,
    mismatch: SeccompAction,
) -> Result<SeccompFilter, seccompiler::BackendError> {
    use std::collections::BTreeMap;
    let mut rules = BTreeMap::new();
    for number in unconditional_syscalls() {
        rules.insert(number, Vec::new());
    }
    // New executable mappings would let a later dlopen in. Code already mapped stays.
    let no_exec = SeccompRule::new(vec![SeccompCondition::new(
        2,
        SeccompCmpArgLen::Dword,
        SeccompCmpOp::MaskedEq(libc::PROT_EXEC as u64),
        0,
    )?])?;
    rules.insert(libc::SYS_mmap, vec![no_exec.clone()]);
    rules.insert(libc::SYS_mprotect, vec![no_exec]);
    SeccompFilter::new(rules, mismatch, SeccompAction::Allow, arch)
}

#[cfg(test)]
pub(super) fn syscall_numbers() -> Vec<i64> {
    let mut numbers = unconditional_syscalls();
    numbers.push(libc::SYS_mmap);
    numbers.push(libc::SYS_mprotect);
    numbers
}

/// Calls a login, a letter and a save used after the filter, plus exit and the timers a
/// short session never reached. prctl, clone and ioctl are not argument-filtered.
fn unconditional_syscalls() -> Vec<i64> {
    let mut numbers = vec![
        libc::SYS_accept4,
        libc::SYS_bind,
        libc::SYS_brk,
        libc::SYS_clock_gettime,
        libc::SYS_clock_nanosleep,
        libc::SYS_clone,
        libc::SYS_clone3,
        libc::SYS_close,
        libc::SYS_close_range,
        libc::SYS_connect,
        libc::SYS_dup,
        libc::SYS_dup3,
        libc::SYS_epoll_create1,
        libc::SYS_epoll_ctl,
        libc::SYS_epoll_pwait,
        libc::SYS_epoll_pwait2,
        libc::SYS_eventfd2,
        libc::SYS_exit,
        libc::SYS_exit_group,
        libc::SYS_faccessat,
        libc::SYS_faccessat2,
        libc::SYS_fallocate,
        libc::SYS_fcntl,
        libc::SYS_fdatasync,
        libc::SYS_fstat,
        libc::SYS_fstatfs,
        libc::SYS_fsync,
        libc::SYS_ftruncate,
        libc::SYS_futex,
        libc::SYS_getcwd,
        libc::SYS_getdents64,
        libc::SYS_geteuid,
        libc::SYS_getgid,
        libc::SYS_getpeername,
        libc::SYS_getpid,
        libc::SYS_getrandom,
        libc::SYS_getrlimit,
        libc::SYS_getsockname,
        libc::SYS_getsockopt,
        libc::SYS_gettid,
        libc::SYS_gettimeofday,
        libc::SYS_getuid,
        libc::SYS_ioctl,
        libc::SYS_listen,
        libc::SYS_lseek,
        libc::SYS_madvise,
        libc::SYS_membarrier,
        libc::SYS_mkdirat,
        libc::SYS_mremap,
        libc::SYS_munmap,
        libc::SYS_nanosleep,
        libc::SYS_newfstatat,
        libc::SYS_openat,
        libc::SYS_openat2,
        libc::SYS_pipe2,
        libc::SYS_ppoll,
        libc::SYS_prctl,
        libc::SYS_pread64,
        libc::SYS_prlimit64,
        libc::SYS_pselect6,
        libc::SYS_pwrite64,
        libc::SYS_read,
        libc::SYS_readlinkat,
        libc::SYS_readv,
        libc::SYS_recvfrom,
        libc::SYS_recvmsg,
        libc::SYS_renameat2,
        libc::SYS_restart_syscall,
        libc::SYS_rseq,
        libc::SYS_rt_sigaction,
        libc::SYS_rt_sigprocmask,
        libc::SYS_rt_sigreturn,
        libc::SYS_sched_getaffinity,
        libc::SYS_sched_setaffinity,
        libc::SYS_sched_yield,
        libc::SYS_sendmsg,
        libc::SYS_sendto,
        libc::SYS_set_robust_list,
        libc::SYS_set_tid_address,
        libc::SYS_setrlimit,
        libc::SYS_setsockopt,
        libc::SYS_shutdown,
        libc::SYS_sigaltstack,
        libc::SYS_signalfd4,
        libc::SYS_socket,
        libc::SYS_socketpair,
        libc::SYS_statfs,
        libc::SYS_statx,
        libc::SYS_tgkill,
        libc::SYS_timerfd_create,
        libc::SYS_timerfd_gettime,
        libc::SYS_timerfd_settime,
        libc::SYS_uname,
        libc::SYS_unlinkat,
        libc::SYS_write,
        libc::SYS_writev,
    ];
    // glibc on x86_64 still issues these legacy names. aarch64 has no such constants.
    #[cfg(target_arch = "x86_64")]
    numbers.extend([
        libc::SYS_access,
        libc::SYS_arch_prctl,
        libc::SYS_epoll_wait,
        libc::SYS_mkdir,
        libc::SYS_poll,
        libc::SYS_unlink,
    ]);
    numbers
}

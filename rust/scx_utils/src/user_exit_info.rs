// Copyright (c) Meta Platforms, Inc. and affiliates.
//
// This software may be used and distributed according to the terms of the
// GNU General Public License version 2.
use crate::bindings;
use anyhow::bail;
use anyhow::Result;
use std::ffi::CStr;
use std::os::raw::c_char;
use std::sync::Mutex;

pub struct UeiDumpPtr {
    pub ptr: *const c_char,
}
unsafe impl Send for UeiDumpPtr {}

pub static UEI_DUMP_PTR_MUTEX: Mutex<UeiDumpPtr> = Mutex::new(UeiDumpPtr {
    ptr: std::ptr::null(),
});

pub enum ScxExitKind {
    None = bindings::scx_exit_kind_SCX_EXIT_NONE as isize,
    Done = bindings::scx_exit_kind_SCX_EXIT_DONE as isize,
    Unreg = bindings::scx_exit_kind_SCX_EXIT_UNREG as isize,
    SysRq = bindings::scx_exit_kind_SCX_EXIT_SYSRQ as isize,
    Error = bindings::scx_exit_kind_SCX_EXIT_ERROR as isize,
    ErrorBPF = bindings::scx_exit_kind_SCX_EXIT_ERROR_BPF as isize,
    ErrorStall = bindings::scx_exit_kind_SCX_EXIT_ERROR_STALL as isize,
}

pub enum ScxInternalConsts {
    ExitDumpDflLen = bindings::scx_internal_consts_SCX_EXIT_DUMP_DFL_LEN as isize,
}

/// Takes a reference to C struct user_exit_info and reads it into
/// UserExitInfo. See UserExitInfo.
#[macro_export]
macro_rules! uei_read {
    ($skel: expr, $uei:ident) => {{
        scx_utils::paste! {
            let bpf_uei = $skel.data().$uei;
            let bpf_dump = scx_utils::UEI_DUMP_PTR_MUTEX.lock().unwrap().ptr;

            scx_utils::UserExitInfo::new(
                &bpf_uei.kind as *const _,
                bpf_uei.reason.as_ptr() as *const _,
                bpf_uei.msg.as_ptr() as *const _,
                bpf_dump,
            )
        }
    }};
}

/// Resize debug dump area according to ops.exit_dump_len. If this macro is
/// not called, debug dump area is not allocated and debug dump won't be
/// printed out.
#[macro_export]
macro_rules! uei_set_size {
    ($skel: expr, $ops: ident, $uei:ident) => {{
        scx_utils::paste! {
            let len = match $skel.struct_ops.$ops().exit_dump_len {
                0 => scx_utils::ScxInternalConsts::ExitDumpDflLen as u32,
                v => v,
            };
            $skel.rodata_mut().[<$uei _dump_len>] = len;
            $skel.maps_mut().[<data_ $uei _dump>]().set_value_size(len).unwrap();

            let mut ptr = scx_utils::UEI_DUMP_PTR_MUTEX.lock().unwrap();
            *ptr = scx_utils::UeiDumpPtr { ptr:
                       $skel
                       .maps()
                       .[<data_ $uei _dump>]()
                       .initial_value()
                       .unwrap()
                       .as_ptr() as *const _,
            };
        }
    }};
}

/// Takes a reference to C struct user_exit_info and test whether the BPF
/// scheduler has exited. See UserExitInfo.
#[macro_export]
macro_rules! uei_exited {
    ($skel: expr, $uei:ident) => {{
        let bpf_uei = $skel.data().uei;
        (unsafe { std::ptr::read_volatile(&bpf_uei.kind as *const _) } != 0)
    }};
}

/// Takes a reference to C struct user_exit_info, reads it and invokes
/// UserExitInfo::report() on it. See UserExitInfo.
#[macro_export]
macro_rules! uei_report {
    ($skel: expr, $uei:ident) => {{
        scx_utils::uei_read!($skel, $uei).report()
    }};
}

/// Rust counterpart of C struct user_exit_info.
#[derive(Debug, Default)]
pub struct UserExitInfo {
    /// The C enum scx_exit_kind value. Test against ScxExitKind. None-zero
    /// value indicates that the BPF scheduler has exited.
    kind: i32,
    reason: Option<String>,
    msg: Option<String>,
    dump: Option<String>,
}

impl UserExitInfo {
    /// Create UserExitInfo from C struct user_exit_info. Each scheduler
    /// implementation creates its own Rust binding for the C struct
    /// user_exit_info, so we can't take the type directly. Instead, this
    /// method takes each member field. Use the macro uei_read!() on the C
    /// type which then calls this method with the individual fields.
    pub fn new(
        kind_ptr: *const i32,
        reason_ptr: *const c_char,
        msg_ptr: *const c_char,
        dump_ptr: *const c_char,
    ) -> Self {
        let kind = unsafe { std::ptr::read_volatile(kind_ptr) };

        let (reason, msg) = (
            Some(
                unsafe { CStr::from_ptr(reason_ptr) }
                    .to_str()
                    .expect("Failed to convert reason to string")
                    .to_string(),
            )
            .filter(|s| !s.is_empty()),
            Some(
                unsafe { CStr::from_ptr(msg_ptr) }
                    .to_str()
                    .expect("Failed to convert msg to string")
                    .to_string(),
            )
            .filter(|s| !s.is_empty()),
        );

        let dump = if dump_ptr.is_null() {
            None
        } else {
            Some(
                unsafe { CStr::from_ptr(dump_ptr) }
                    .to_str()
                    .expect("Failed to convert msg to string")
                    .to_string(),
            )
            .filter(|s| !s.is_empty())
        };

        Self {
            kind,
            reason,
            msg,
            dump,
        }
    }

    /// Print out the exit message to stderr if the exit was normal. After
    /// an error exit, it throws an error containing the exit message
    /// instead. If debug dump exists, it's always printed to stderr.
    pub fn report(&self) -> Result<()> {
        if self.kind == 0 {
            return Ok(());
        }

        if let Some(dump) = &self.dump {
            eprintln!("\nDEBUG DUMP");
            eprintln!("================================================================================\n");
            eprintln!("{}", dump);
            eprintln!("================================================================================\n");
        }

        let why = match (&self.reason, &self.msg) {
            (Some(reason), None) => format!("EXIT: {}", reason),
            (Some(reason), Some(msg)) => format!("EXIT: {} ({})", reason, msg),
            _ => "<UNKNOWN>".into(),
        };

        if self.kind <= ScxExitKind::Unreg as i32 {
            eprintln!("{}", why);
            Ok(())
        } else {
            bail!("{}", why)
        }
    }
}

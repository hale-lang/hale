//! `std::process::*` path-call lowering.

use hale_syntax::ast::Expr;
use inkwell::values::{BasicValueEnum, PointerValue};
use inkwell::AddressSpace;

use crate::codegen::{
    CodegenError, CodegenTy, Cx, FallibleCallResult, Scope, TypeInfo,
};

pub(crate) trait ProcessStdlib<'ctx> {
    fn lower_std_process_exit(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(), CodegenError>;

    fn lower_std_process_pid(
        &mut self,
        args: &[Expr],
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;

    fn lower_std_process_uid(
        &mut self,
        args: &[Expr],
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;

    fn lower_std_process_rss_bytes(
        &mut self,
        args: &[Expr],
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;

    fn lower_std_process_dump_arena_residency(
        &mut self,
        args: &[Expr],
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;

    fn lower_std_process_dump_pool_residency(
        &mut self,
        args: &[Expr],
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;

    fn lower_std_process_run_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError>;

    fn lower_std_process_spawn_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError>;

    fn lower_std_process_run_with_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError>;

    fn lower_std_process_spawn_with_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError>;

    fn lower_std_process_wait_deadline_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError>;

    fn lower_std_process_wait_pid_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError>;

    fn lower_std_process_kill_escalate_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError>;
    fn lower_std_process_try_wait_pid_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError>;
    fn lower_std_process_signal_pid_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError>;

    fn lower_std_process_pipe_read_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError>;

    fn lower_std_process_pipe_write_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError>;

    fn load_i32_to_i64(
        &self,
        slot: PointerValue<'ctx>,
        label: &str,
    ) -> Result<inkwell::values::IntValue<'ctx>, CodegenError>;

    fn store_struct_field(
        &self,
        info: &TypeInfo<'ctx>,
        struct_ptr: PointerValue<'ctx>,
        field: &str,
        val: BasicValueEnum<'ctx>,
        struct_label: &str,
    ) -> Result<(), CodegenError>;
}

impl<'ctx, 'p> ProcessStdlib<'ctx> for Cx<'ctx, 'p> {
    /// Lower `std::process::exit(code: Int)` to libc `exit()`.
    /// Statement-position only; the block becomes terminated
    /// after the call, so we open a fresh basic block to land
    /// any subsequent (dead) lowering into. Matches the
    /// closure-violation handler's exit pattern.
    fn lower_std_process_exit(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::process::exit takes 1 arg (code), got {}",
                args.len()
            )));
        }
        let (code_val, code_ty) = self.lower_expr(&args[0], scope)?;
        if code_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::process::exit: code must be Int, got {:?}",
                code_ty
            )));
        }
        let i32_t = self.context.i32_type();
        let code_i32 = self
            .builder
            .build_int_truncate(code_val.into_int_value(), i32_t, "exit.code.i32")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let exit_fn = self
            .module
            .get_function("exit")
            .expect("exit declared in declare_builtins");
        self.builder
            .build_call(exit_fn, &[code_i32.into()], "exit.call")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_unreachable()
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        // Fresh dead block so any post-exit() statements have
        // somewhere to lower into without violating LLVM's
        // single-terminator-per-block rule.
        let func = self
            .current_fn
            .expect("current_fn set while lowering std::process::exit");
        let after = self.context.append_basic_block(func, "after.exit");
        self.builder.position_at_end(after);
        Ok(())
    }

    /// Lower `std::process::pid() -> Int` to `getpid()`. POSIX
    /// returns `pid_t` (i32 on Linux); Hale `Int` is i64, so we
    /// sign-extend. m71 ships this as the proof symbol that the
    /// magic-`std::*`-path resolver works end-to-end; the same
    /// pattern (declare libc fn → match arm → one `lower_std_*`
    /// method) extends to every Phase 1 stdlib function.
    fn lower_std_process_pid(
        &mut self,
        args: &[Expr],
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if !args.is_empty() {
            return Err(CodegenError::Unsupported(format!(
                "std::process::pid takes 0 arguments, got {}",
                args.len()
            )));
        }
        let i64_t = self.context.i64_type();
        let getpid = self
            .module
            .get_function("getpid")
            .expect("getpid declared");
        let call = self
            .builder
            .build_call(getpid, &[], "getpid.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let pid_i32 = call
            .try_as_basic_value()
            .left()
            .expect("getpid returns i32")
            .into_int_value();
        let pid_i64 = self
            .builder
            .build_int_s_extend(pid_i32, i64_t, "pid.i64")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok((pid_i64.into(), CodegenTy::Int))
    }

    /// `std::process::uid() -> Int`: `getuid()`, sign-extended like
    /// `pid`. The api binding asks it to know a peer is the program
    /// itself (a forwarding transport's `via` mark, GH #1104).
    fn lower_std_process_uid(
        &mut self,
        args: &[Expr],
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if !args.is_empty() {
            return Err(CodegenError::Unsupported(format!(
                "std::process::uid takes 0 arguments, got {}",
                args.len()
            )));
        }
        let i64_t = self.context.i64_type();
        let getuid = self
            .module
            .get_function("getuid")
            .expect("getuid declared");
        let call = self
            .builder
            .build_call(getuid, &[], "getuid.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let uid_i32 = call
            .try_as_basic_value()
            .left()
            .expect("getuid returns i32")
            .into_int_value();
        let uid_i64 = self
            .builder
            .build_int_z_extend(uid_i32, i64_t, "uid.i64")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok((uid_i64.into(), CodegenTy::Int))
    }

    /// `std::process::rss_bytes() -> Int` (2026-05-21). Returns
    /// the calling process's peak resident-set size in bytes via
    /// `getrusage(RUSAGE_SELF)`. Observability primitive — lets
    /// a long-running daemon verify the Phase-4 method-scratch
    /// reclaim actually bounds memory (the previous attempt to
    /// read /proc/self/statm via `std::io::fs::read_file` hit
    /// the synthesized-file fstat-returns-0 bug). Peak (not
    /// current) RSS is what getrusage exposes; alarm thresholds
    /// typically want the worst-case anyway.
    fn lower_std_process_rss_bytes(
        &mut self,
        args: &[Expr],
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if !args.is_empty() {
            return Err(CodegenError::Unsupported(format!(
                "std::process::rss_bytes takes 0 arguments, got {}",
                args.len()
            )));
        }
        let f = self
            .module
            .get_function("lotus_process_rss_bytes")
            .expect("lotus_process_rss_bytes declared");
        let bytes = self
            .builder
            .build_call(f, &[], "rss_bytes.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("lotus_process_rss_bytes returns i64");
        Ok((bytes, CodegenTy::Int))
    }

    /// `std::process::dump_arena_residency()` (2026-05-22 PM).
    /// Writes per-arena residency snapshot (bytes / chunks /
    /// construction backtrace) to stderr. No-op unless the
    /// program was started with `LOTUS_ARENA_RESIDENCY=1`. The
    /// dump runs synchronously and walks the live registry under
    /// a mutex; safe to call from any thread, but reserve for
    /// stats-emit / checkpoint hooks (locus dissolve at scope
    /// exit will hide the residency from later snapshots, which
    /// is why an atexit hook alone won't catch a long-running
    /// daemon's locus arenas — downstream apps call this from its
    /// checkpoint tick).
    fn lower_std_process_dump_arena_residency(
        &mut self,
        args: &[Expr],
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if !args.is_empty() {
            return Err(CodegenError::Unsupported(format!(
                "std::process::dump_arena_residency takes 0 arguments, got {}",
                args.len()
            )));
        }
        let f = self
            .module
            .get_function("lotus_arena_residency_dump_fd")
            .expect("lotus_arena_residency_dump_fd declared");
        // fd=2 → stderr. Matches the diagnostic conventions of
        // LOTUS_ARENA_LOG_BIG_CHUNKS and LOTUS_CHUNK_POOL_STATS.
        let i32_t = self.context.i32_type();
        let stderr_fd = i32_t.const_int(2, false);
        self.builder
            .build_call(f, &[stderr_fd.into()], "arena_residency.dump")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        // Void return — surface as `Bool(true)` so the surface is
        // call-as-expression compatible without forcing a stmt-only
        // syntax. Matches the cheap "returns true" pattern used by
        // a few other observability primitives. Actually return
        // unit-like Int(0) to avoid implying a meaningful value.
        let i64_t = self.context.i64_type();
        Ok((i64_t.const_int(0, false).into(), CodegenTy::Int))
    }

    /// F.35 Slice 4: `std::process::dump_pool_residency()` —
    /// writes one line per cooperative pool to stderr naming
    /// the pool's name, I/O mode (async_io / blocking), parked-
    /// coro count, and pending cell-queue depth. Mirrors the
    /// `dump_arena_residency` shape — call from a heartbeat tick
    /// on long-running daemons to track per-pool occupancy. The
    /// dump is cheap (one mutex acquire per pool); the parked-
    /// list walk reads-without-lock because only the pool's own
    /// worker mutates that list.
    fn lower_std_process_dump_pool_residency(
        &mut self,
        args: &[Expr],
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if !args.is_empty() {
            return Err(CodegenError::Unsupported(format!(
                "std::process::dump_pool_residency takes 0 arguments, got {}",
                args.len()
            )));
        }
        let f = self
            .module
            .get_function("lotus_coop_pool_dump_parked_counts")
            .expect("lotus_coop_pool_dump_parked_counts declared");
        let i32_t = self.context.i32_type();
        let stderr_fd = i32_t.const_int(2, false);
        self.builder
            .build_call(f, &[stderr_fd.into()], "pool_residency.dump")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let i64_t = self.context.i64_type();
        Ok((i64_t.const_int(0, false).into(), CodegenTy::Int))
    }

    /// C2 — `std::process::run(argv: String) -> ProcessOutput
    /// fallible(IoError)`. Synchronous fork+exec+wait. The C
    /// primitive populates four out-pointers (code, signal,
    /// stdout-ptr, stderr-ptr); we allocate a fresh
    /// `__StdProcessOutput` struct in the locus arena, store the
    /// four field values, and return the struct pointer as the
    /// success-arm value.
    ///
    /// On failure (non-zero errno from `lotus_process_run`), the
    /// C primitive sets errno before returning so the IoError
    /// kind tag picks up the appropriate label
    /// ("not_found" / "permission_denied" / "invalid" / etc.).
    /// The diagnostic-path on IoError is the surface label
    /// "std::process::run" — there's no caller-supplied path
    /// here, but the agent still benefits from seeing which
    /// surface raised.
    fn lower_std_process_run_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        self.lower_process_run(args, scope, false)
    }

    /// `std::process::__run_with(argv, cwd, env, unset_env, clear_env,
    /// timeout_ns, cpu_seconds, address_space, open_files, file_size)
    /// -> __StdProcessOutput fallible(IoError)` — `run` under a
    /// `SpawnOptions`, the primitive `process.hl`'s `run_with` unpacks
    /// the record into. The result's `timed_out` says the deadline
    /// stopped the child.
    fn lower_std_process_run_with_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        self.lower_process_run(args, scope, true)
    }

    /// `std::process::__spawn_with(argv, cwd, env, unset_env,
    /// clear_env, timeout_ns, cpu_seconds, address_space, open_files,
    /// file_size) -> __StdProcessSpawnHandle fallible(IoError)` — the
    /// handle carries the deadline `wait` / `try_wait` enforce.
    fn lower_std_process_spawn_with_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        self.lower_process_spawn(args, scope, true)
    }

    /// `std::process::__wait_deadline(pid, deadline_ns, nonblocking)
    /// -> __StdProcessWaitOutcome fallible(IoError)` — `wait` /
    /// `try_wait` on a handle with a deadline: code -2 = still running
    /// (nonblocking), -3 = stopped at the deadline.
    fn lower_std_process_wait_deadline_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        if args.len() != 3 {
            return Err(CodegenError::Unsupported(format!(
                "std::process::__wait_deadline takes 3 args (pid, deadline_ns, \
                 nonblocking: Int), got {}",
                args.len()
            )));
        }
        let mut ints = Vec::with_capacity(3);
        for a in args {
            let (v, ty) = self.lower_expr(a, scope)?;
            if ty != CodegenTy::Int {
                return Err(CodegenError::Unsupported(format!(
                    "std::process::__wait_deadline: arguments are Int, got {:?}",
                    ty
                )));
            }
            ints.push(v.into_int_value());
        }
        let i32_t = self.context.i32_type();
        let pid_i32 = self
            .builder
            .build_int_truncate(ints[0], i32_t, "proc.waitdl.pid.i32")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let code_slot = self.alloca_in_entry(i32_t.into(), "proc.waitdl.code.slot")?;
        let sig_slot = self.alloca_in_entry(i32_t.into(), "proc.waitdl.sig.slot")?;
        let f = self
            .module
            .get_function("lotus_process_wait_deadline")
            .expect("lotus_process_wait_deadline declared");
        let ret_i32 = self
            .builder
            .build_call(
                f,
                &[
                    pid_i32.into(),
                    ints[1].into(),
                    ints[2].into(),
                    code_slot.into(),
                    sig_slot.into(),
                ],
                "proc.waitdl.ret",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns i32")
            .into_int_value();
        let is_err = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::NE,
                ret_i32,
                i32_t.const_zero(),
                "proc.waitdl.is_err",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let info = self
            .user_types
            .get("__StdProcessWaitOutcome")
            .cloned()
            .expect("__StdProcessWaitOutcome declared in process.hl");
        let size = info
            .struct_ty
            .size_of()
            .expect("__StdProcessWaitOutcome has known size");
        let struct_ptr = self.arena_alloc(size, "WaitDeadlineOutcome.alloc")?;
        let code_i64 = self.load_i32_to_i64(code_slot, "proc.waitdl.code.val")?;
        let sig_i64 = self.load_i32_to_i64(sig_slot, "proc.waitdl.sig.val")?;
        self.store_struct_field(
            &info, struct_ptr, "code", code_i64.into(), "WaitOutcome",
        )?;
        self.store_struct_field(
            &info, struct_ptr, "signal", sig_i64.into(), "WaitOutcome",
        )?;
        let path_label = self
            .builder
            .build_global_string_ptr("std::process::wait", "proc.waitdl.label")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .as_pointer_value();
        self.complete_io_fallible_call(
            is_err,
            path_label.into(),
            Some((struct_ptr.into(), CodegenTy::TypeRef(
                "__StdProcessWaitOutcome".to_string()
            ))),
            "proc.waitdl",
        )
    }

    /// C2 — `std::process::__spawn(argv: String) ->
    /// __StdProcessSpawnHandle fallible(IoError)`. Internal
    /// primitive consumed by `process.hl`'s `spawn()` wrapper.
    fn lower_std_process_spawn_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        self.lower_process_spawn(args, scope, false)
    }

    /// C2 — `std::process::__wait_pid(pid: Int) ->
    /// __StdProcessWaitOutcome fallible(IoError)`. Internal
    /// primitive consumed by `process.hl`'s `wait()` wrapper.
    fn lower_std_process_wait_pid_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::process::__wait_pid takes 1 arg (pid: Int), got {}",
                args.len()
            )));
        }
        let (pid_val, pid_ty) = self.lower_expr(&args[0], scope)?;
        if pid_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::process::__wait_pid: pid must be Int, got {:?}",
                pid_ty
            )));
        }
        let i32_t = self.context.i32_type();
        let pid_i32 = self
            .builder
            .build_int_truncate(pid_val.into_int_value(), i32_t, "proc.wait.pid.i32")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        let code_slot = self.alloca_in_entry(i32_t.into(), "proc.wait.code.slot")?;
        let sig_slot = self.alloca_in_entry(i32_t.into(), "proc.wait.sig.slot")?;
        let f = self
            .module
            .get_function("lotus_process_wait")
            .expect("lotus_process_wait declared");
        let ret_i32 = self
            .builder
            .build_call(
                f,
                &[pid_i32.into(), code_slot.into(), sig_slot.into()],
                "proc.wait.ret",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns i32")
            .into_int_value();
        let is_err = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::NE,
                ret_i32,
                i32_t.const_zero(),
                "proc.wait.is_err",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        let info = self
            .user_types
            .get("__StdProcessWaitOutcome")
            .cloned()
            .expect("__StdProcessWaitOutcome declared in process.hl");
        let size = info
            .struct_ty
            .size_of()
            .expect("__StdProcessWaitOutcome has known size");
        let struct_ptr = self.arena_alloc(size, "WaitOutcome.alloc")?;

        let code_i64 = self.load_i32_to_i64(code_slot, "proc.wait.code.val")?;
        let sig_i64 = self.load_i32_to_i64(sig_slot, "proc.wait.sig.val")?;
        self.store_struct_field(
            &info, struct_ptr, "code", code_i64.into(), "WaitOutcome",
        )?;
        self.store_struct_field(
            &info, struct_ptr, "signal", sig_i64.into(), "WaitOutcome",
        )?;

        let path_label = self
            .builder
            .build_global_string_ptr("std::process::wait", "proc.wait.label")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .as_pointer_value();
        self.complete_io_fallible_call(
            is_err,
            path_label.into(),
            Some((struct_ptr.into(), CodegenTy::TypeRef(
                "__StdProcessWaitOutcome".to_string()
            ))),
            "proc.wait",
        )
    }

    /// try_wait (2026-07-17): non-blocking sibling of __wait_pid.
    /// Same WaitOutcome shape; the C side reports a still-running
    /// child as code = -2 (the stdlib retryable sentinel).
    fn lower_std_process_try_wait_pid_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::process::__try_wait_pid takes 1 arg (pid: Int), got {}",
                args.len()
            )));
        }
        let (pid_val, pid_ty) = self.lower_expr(&args[0], scope)?;
        if pid_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::process::__try_wait_pid: pid must be Int, got {:?}",
                pid_ty
            )));
        }
        let i32_t = self.context.i32_type();
        let pid_i32 = self
            .builder
            .build_int_truncate(pid_val.into_int_value(), i32_t, "proc.trywait.pid.i32")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        let code_slot = self.alloca_in_entry(i32_t.into(), "proc.trywait.code.slot")?;
        let sig_slot = self.alloca_in_entry(i32_t.into(), "proc.trywait.sig.slot")?;
        let f = self
            .module
            .get_function("lotus_process_try_wait")
            .expect("lotus_process_try_wait declared");
        let ret_i32 = self
            .builder
            .build_call(
                f,
                &[pid_i32.into(), code_slot.into(), sig_slot.into()],
                "proc.trywait.ret",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns i32")
            .into_int_value();
        let is_err = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::NE,
                ret_i32,
                i32_t.const_zero(),
                "proc.trywait.is_err",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        let info = self
            .user_types
            .get("__StdProcessWaitOutcome")
            .cloned()
            .expect("__StdProcessWaitOutcome declared in process.hl");
        let size = info
            .struct_ty
            .size_of()
            .expect("__StdProcessWaitOutcome has known size");
        let struct_ptr = self.arena_alloc(size, "TryWaitOutcome.alloc")?;

        let code_i64 = self.load_i32_to_i64(code_slot, "proc.trywait.code.val")?;
        let sig_i64 = self.load_i32_to_i64(sig_slot, "proc.trywait.sig.val")?;
        self.store_struct_field(
            &info, struct_ptr, "code", code_i64.into(), "WaitOutcome",
        )?;
        self.store_struct_field(
            &info, struct_ptr, "signal", sig_i64.into(), "WaitOutcome",
        )?;

        let path_label = self
            .builder
            .build_global_string_ptr("std::process::try_wait", "proc.trywait.label")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .as_pointer_value();
        self.complete_io_fallible_call(
            is_err,
            path_label.into(),
            Some((struct_ptr.into(), CodegenTy::TypeRef(
                "__StdProcessWaitOutcome".to_string()
            ))),
            "proc.trywait",
        )
    }

    /// signal (2026-07-17, promoted from pond/subprocess): send an
    /// arbitrary signal to the child's pid. Unit success; ESRCH
    /// surfaces via the IoError channel.
    fn lower_std_process_signal_pid_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        if args.len() != 2 {
            return Err(CodegenError::Unsupported(format!(
                "std::process::__signal_pid takes 2 args (pid: Int, sig: Int), got {}",
                args.len()
            )));
        }
        let (pid_val, pid_ty) = self.lower_expr(&args[0], scope)?;
        if pid_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::process::__signal_pid: pid must be Int, got {:?}",
                pid_ty
            )));
        }
        let (sig_val, sig_ty) = self.lower_expr(&args[1], scope)?;
        if sig_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::process::__signal_pid: sig must be Int, got {:?}",
                sig_ty
            )));
        }
        let i32_t = self.context.i32_type();
        let pid_i32 = self
            .builder
            .build_int_truncate(pid_val.into_int_value(), i32_t, "proc.signal.pid.i32")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let sig_i32 = self
            .builder
            .build_int_truncate(sig_val.into_int_value(), i32_t, "proc.signal.sig.i32")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let f = self
            .module
            .get_function("lotus_process_signal")
            .expect("lotus_process_signal declared");
        let ret_i32 = self
            .builder
            .build_call(f, &[pid_i32.into(), sig_i32.into()], "proc.signal.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns i32")
            .into_int_value();
        let is_err = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::NE,
                ret_i32,
                i32_t.const_zero(),
                "proc.signal.is_err",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let path_label = self
            .builder
            .build_global_string_ptr("std::process::signal", "proc.signal.label")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .as_pointer_value();
        self.complete_io_fallible_call(
            is_err,
            path_label.into(),
            None,
            "proc.signal",
        )
    }

    /// C2 — `std::process::__kill_escalate(pid: Int) -> ()
    /// fallible(IoError)`. SIGTERM → 100ms grace → SIGKILL →
    /// waitpid. Internal primitive consumed by `process.hl`'s
    /// `kill()` wrapper.
    fn lower_std_process_kill_escalate_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::process::__kill_escalate takes 1 arg (pid: Int), got {}",
                args.len()
            )));
        }
        let (pid_val, pid_ty) = self.lower_expr(&args[0], scope)?;
        if pid_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::process::__kill_escalate: pid must be Int, got {:?}",
                pid_ty
            )));
        }
        let i32_t = self.context.i32_type();
        let pid_i32 = self
            .builder
            .build_int_truncate(pid_val.into_int_value(), i32_t, "proc.kill.pid.i32")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let f = self
            .module
            .get_function("lotus_process_kill_escalate")
            .expect("lotus_process_kill_escalate declared");
        let ret_i32 = self
            .builder
            .build_call(f, &[pid_i32.into()], "proc.kill.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns i32")
            .into_int_value();
        let is_err = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::NE,
                ret_i32,
                i32_t.const_zero(),
                "proc.kill.is_err",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let path_label = self
            .builder
            .build_global_string_ptr("std::process::kill", "proc.kill.label")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .as_pointer_value();
        self.complete_io_fallible_call(
            is_err,
            path_label.into(),
            None,
            "proc.kill",
        )
    }

    /// C2 — `std::process::__pipe_read(fd: Int) -> String
    /// fallible(IoError)`. Non-blocking read of up to 64 KiB from
    /// the pipe fd. Returns empty String on EAGAIN/EOF, hard
    /// error on EBADF/EIO.
    fn lower_std_process_pipe_read_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::process::__pipe_read takes 1 arg (fd: Int), got {}",
                args.len()
            )));
        }
        let (fd_val, fd_ty) = self.lower_expr(&args[0], scope)?;
        if fd_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::process::__pipe_read: fd must be Int, got {:?}",
                fd_ty
            )));
        }
        let i32_t = self.context.i32_type();
        let i64_t = self.context.i64_type();
        let fd_i32 = self
            .builder
            .build_int_truncate(fd_val.into_int_value(), i32_t, "proc.pread.fd.i32")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let f = self
            .module
            .get_function("lotus_process_pipe_read_nonblocking")
            .expect("lotus_process_pipe_read_nonblocking declared");
        // F.8 sweep — see lower_std_str_builder_finish for rationale.
        self.emit_set_caller_arena()?;
        let ret_ptr = self
            .builder
            .build_call(f, &[fd_i32.into()], "proc.pread.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns ptr")
            .into_pointer_value();
        // NULL → hard error.
        let is_err = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::EQ,
                self.builder
                    .build_ptr_to_int(ret_ptr, i64_t, "proc.pread.as_int")
                    .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?,
                i64_t.const_zero(),
                "proc.pread.is_err",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let path_label = self
            .builder
            .build_global_string_ptr(
                "std::process::pipe_read",
                "proc.pread.label",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .as_pointer_value();
        self.complete_io_fallible_call(
            is_err,
            path_label.into(),
            Some((ret_ptr.into(), CodegenTy::String)),
            "proc.pread",
        )
    }

    /// C2 — `std::process::__pipe_write(fd: Int, s: String) ->
    /// Int fallible(IoError)`. Blocking write of the full string;
    /// returns bytes written. SIGPIPE-driven crashes are
    /// suppressed by the global SIG_IGN in lotus_io_init — a
    /// closed-pipe write surfaces as EPIPE through the IoError
    /// channel.
    fn lower_std_process_pipe_write_fallible(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        if args.len() != 2 {
            return Err(CodegenError::Unsupported(format!(
                "std::process::__pipe_write takes 2 args (fd: Int, s: String), got {}",
                args.len()
            )));
        }
        let (fd_val, fd_ty) = self.lower_expr(&args[0], scope)?;
        if fd_ty != CodegenTy::Int {
            return Err(CodegenError::Unsupported(format!(
                "std::process::__pipe_write: fd must be Int, got {:?}",
                fd_ty
            )));
        }
        let (s_val, s_ty) = self.lower_expr(&args[1], scope)?;
        if !matches!(s_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "std::process::__pipe_write: s must be String, got {:?}",
                s_ty
            )));
        }
        let s_val = self.unpack_view_if_needed(s_val, &s_ty)?;
        let i32_t = self.context.i32_type();
        let i64_t = self.context.i64_type();
        let fd_i32 = self
            .builder
            .build_int_truncate(fd_val.into_int_value(), i32_t, "proc.pwrite.fd.i32")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let f = self
            .module
            .get_function("lotus_process_pipe_write")
            .expect("lotus_process_pipe_write declared");
        let ret_i64 = self
            .builder
            .build_call(f, &[fd_i32.into(), s_val.into()], "proc.pwrite.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns i64")
            .into_int_value();
        // -1 → error.
        let is_err = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::SLT,
                ret_i64,
                i64_t.const_zero(),
                "proc.pwrite.is_err",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let path_label = self
            .builder
            .build_global_string_ptr(
                "std::process::pipe_write",
                "proc.pwrite.label",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .as_pointer_value();
        self.complete_io_fallible_call(
            is_err,
            path_label.into(),
            Some((ret_i64.into(), CodegenTy::Int)),
            "proc.pwrite",
        )
    }

    /// C2 helper: load an i32 from a slot and sign-extend to i64.
    /// Used by the spawn/wait fallible lowerings whose C primitives
    /// write i32 pids/fds/exit-codes that we widen to Hale's Int.
    fn load_i32_to_i64(
        &self,
        slot: PointerValue<'ctx>,
        label: &str,
    ) -> Result<inkwell::values::IntValue<'ctx>, CodegenError> {
        let i32_t = self.context.i32_type();
        let i64_t = self.context.i64_type();
        let v = self
            .builder
            .build_load(i32_t, slot, label)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_int_value();
        self.builder
            .build_int_s_extend(v, i64_t, &format!("{}.i64", label))
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))
    }

    /// C2 helper: store `val` into the struct field named `field`
    /// at `struct_ptr`. `struct_label` is used in the GEP name for
    /// IR readability. Centralizes the GEP+store pattern shared by
    /// the four `lower_std_process_*` lowerings that build a result
    /// struct from out-pointer values.
    fn store_struct_field(
        &self,
        info: &TypeInfo<'ctx>,
        struct_ptr: PointerValue<'ctx>,
        field: &str,
        val: BasicValueEnum<'ctx>,
        struct_label: &str,
    ) -> Result<(), CodegenError> {
        let (idx, _) = info
            .fields
            .get(field)
            .cloned()
            .unwrap_or_else(|| panic!("{}.{} field", struct_label, field));
        let fp = self
            .builder
            .build_struct_gep(
                info.struct_ty,
                struct_ptr,
                idx,
                &format!("{}.{}.ptr", struct_label, field),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_store(fp, val)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok(())
    }
}

impl<'ctx, 'p> Cx<'ctx, 'p> {
    /// hale-bun upstream item 4: emit the plain + hint surface
    /// labels for a run/spawn call and pick between them at
    /// runtime via `lotus_process_enoent_hint_label` — the hint
    /// wins when errno is ENOENT and argv[0] contains a space
    /// (shell-split argv passed to the newline-separated
    /// surface, which otherwise reads as "binary not on PATH").
    /// Must be called right after the failed primitive, while
    /// its errno is still live; the helper reads errno only.
    fn process_surface_label_with_enoent_hint(
        &mut self,
        argv_val: BasicValueEnum<'ctx>,
        plain: &str,
        hint: &str,
        tag: &str,
    ) -> Result<PointerValue<'ctx>, CodegenError> {
        let plain_g = self
            .builder
            .build_global_string_ptr(plain, &format!("{}.label", tag))
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .as_pointer_value();
        let hint_g = self
            .builder
            .build_global_string_ptr(hint, &format!("{}.label.hint", tag))
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .as_pointer_value();
        let f = self
            .module
            .get_function("lotus_process_enoent_hint_label")
            .expect("lotus_process_enoent_hint_label declared");
        let call = self
            .builder
            .build_call(
                f,
                &[argv_val.into(), plain_g.into(), hint_g.into()],
                &format!("{}.label.pick", tag),
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok(call
            .try_as_basic_value()
            .left()
            .expect("returns ptr")
            .into_pointer_value())
    }

    /// `run` (`with_opts` false: one arg) and `__run_with` (ten: argv
    /// and the `SpawnOptions` fields, see `lower_process_option_args`).
    fn lower_process_run(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
        with_opts: bool,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        let (surface, want) = if with_opts {
            ("std::process::run_with", 10)
        } else {
            ("std::process::run", 1)
        };
        if args.len() != want {
            return Err(CodegenError::Unsupported(format!(
                "{} takes {} arg(s), got {}",
                surface,
                want,
                args.len()
            )));
        }
        let (argv_val, argv_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(argv_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "{}: argv must be String (newline-\
                 separated), got {:?}",
                surface, argv_ty
            )));
        }
        let argv_val = self.unpack_view_if_needed(argv_val, &argv_ty)?;
        let opt_vals = if with_opts {
            self.lower_process_option_args(&args[1..], scope, surface)?
        } else {
            Vec::new()
        };
        let i32_t = self.context.i32_type();
        let ptr_t = self.context.ptr_type(AddressSpace::default());

        // Allocate stack slots for the four out-params. Hoisted
        // to the entry block so a `run()` inside a loop doesn't
        // grow the stack per iteration.
        let code_slot = self.alloca_in_entry(i32_t.into(), "proc.run.code.slot")?;
        let sig_slot = self.alloca_in_entry(i32_t.into(), "proc.run.sig.slot")?;
        let out_slot = self.alloca_in_entry(ptr_t.into(), "proc.run.out.slot")?;
        let err_slot = self.alloca_in_entry(ptr_t.into(), "proc.run.err.slot")?;
        let timed_slot = self.alloca_in_entry(i32_t.into(), "proc.run.timed.slot")?;
        self.builder
            .build_store(timed_slot, i32_t.const_zero())
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        let mut call_args: Vec<inkwell::values::BasicMetadataValueEnum<'ctx>> =
            vec![argv_val.into()];
        call_args.extend(opt_vals);
        for slot in [code_slot, sig_slot, out_slot, err_slot] {
            call_args.push(slot.into());
        }
        if with_opts {
            call_args.push(timed_slot.into());
        }
        let run_fn = self
            .module
            .get_function(if with_opts {
                "lotus_process_run_with"
            } else {
                "lotus_process_run"
            })
            .expect("lotus_process_run[_with] declared");
        let ret_i32 = self
            .builder
            .build_call(
                run_fn,
                &call_args,
                "proc.run.ret",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns i32")
            .into_int_value();
        // Non-zero return → fork/exec failure. The C primitive
        // already set errno before returning so the IoError kind
        // tag flows naturally through complete_io_fallible_call.
        let is_err = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::NE,
                ret_i32,
                i32_t.const_zero(),
                "proc.run.is_err",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // On the success arm, build the ProcessOutput struct in
        // the locus arena from the four out-params.
        let info = self
            .user_types
            .get("__StdProcessOutput")
            .cloned()
            .expect("__StdProcessOutput declared in process.hl");
        let size = info
            .struct_ty
            .size_of()
            .expect("__StdProcessOutput has known size");
        let struct_ptr = self.arena_alloc(size, "ProcessOutput.alloc")?;

        // Load each out-param + store into the struct field.
        let i64_t = self.context.i64_type();
        let code_i32_v = self
            .builder
            .build_load(i32_t, code_slot, "proc.run.code.val")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_int_value();
        let code_i64 = self
            .builder
            .build_int_s_extend(code_i32_v, i64_t, "proc.run.code.i64")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let sig_i32_v = self
            .builder
            .build_load(i32_t, sig_slot, "proc.run.sig.val")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_int_value();
        let sig_i64 = self
            .builder
            .build_int_s_extend(sig_i32_v, i64_t, "proc.run.sig.i64")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let stdout_ptr_v = self
            .builder
            .build_load(ptr_t, out_slot, "proc.run.stdout.val")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let stderr_ptr_v = self
            .builder
            .build_load(ptr_t, err_slot, "proc.run.stderr.val")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        self.store_struct_field(
            &info, struct_ptr, "code", code_i64.into(), "ProcessOutput",
        )?;
        self.store_struct_field(
            &info, struct_ptr, "signal", sig_i64.into(), "ProcessOutput",
        )?;
        self.store_struct_field(
            &info, struct_ptr, "stdout", stdout_ptr_v, "ProcessOutput",
        )?;
        self.store_struct_field(
            &info, struct_ptr, "stderr", stderr_ptr_v, "ProcessOutput",
        )?;
        let timed_i32 = self
            .builder
            .build_load(i32_t, timed_slot, "proc.run.timed.val")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_int_value();
        let timed = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::NE,
                timed_i32,
                i32_t.const_zero(),
                "proc.run.timed",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.store_bool_field(&info, struct_ptr, "timed_out", timed, "ProcessOutput")?;

        // Diagnostic-path on IoError = surface label. hale-bun
        // upstream item 4: routed through the C hint helper so an
        // ENOENT with a space in argv[0] (shell-split argv passed
        // to the newline-separated surface) names the real
        // mistake instead of reading as "binary not on PATH".
        // errno is still live from the failed primitive here; on
        // the success arm the label goes unused.
        let path_label = self.process_surface_label_with_enoent_hint(
            argv_val,
            surface,
            &format!(
                "{} — argv[0] contains a space; argv is \
                 newline-separated (\"echo\\nhello\"), not shell-split",
                surface
            ),
            "proc.run",
        )?;
        self.complete_io_fallible_call(
            is_err,
            path_label.into(),
            Some((struct_ptr.into(), CodegenTy::TypeRef(
                "__StdProcessOutput".to_string()
            ))),
            "proc.run",
        )
    }

    /// `__spawn` (`with_opts` false: one arg) and `__spawn_with` (ten:
    /// argv and the `SpawnOptions` fields). The handle's `deadline_ns`
    /// is 0 for `__spawn`, the C side's monotonic deadline for
    /// `__spawn_with` with a timeout.
    fn lower_process_spawn(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
        with_opts: bool,
    ) -> Result<FallibleCallResult<'ctx>, CodegenError> {
        let (surface, want) = if with_opts {
            ("std::process::spawn_with", 10)
        } else {
            ("std::process::spawn", 1)
        };
        if args.len() != want {
            return Err(CodegenError::Unsupported(format!(
                "{} takes {} arg(s), got {}",
                surface,
                want,
                args.len()
            )));
        }
        let (argv_val, argv_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(argv_ty, CodegenTy::String | CodegenTy::StringView) {
            return Err(CodegenError::Unsupported(format!(
                "{}: argv must be String, got {:?}",
                surface, argv_ty
            )));
        }
        let argv_val = self.unpack_view_if_needed(argv_val, &argv_ty)?;
        let opt_vals = if with_opts {
            self.lower_process_option_args(&args[1..], scope, surface)?
        } else {
            Vec::new()
        };
        let i32_t = self.context.i32_type();
        let i64_t = self.context.i64_type();

        let pid_slot = self.alloca_in_entry(i32_t.into(), "proc.spawn.pid.slot")?;
        let in_slot = self.alloca_in_entry(i32_t.into(), "proc.spawn.in.slot")?;
        let out_slot = self.alloca_in_entry(i32_t.into(), "proc.spawn.out.slot")?;
        let err_slot = self.alloca_in_entry(i32_t.into(), "proc.spawn.err.slot")?;
        let deadline_slot =
            self.alloca_in_entry(i64_t.into(), "proc.spawn.deadline.slot")?;
        self.builder
            .build_store(deadline_slot, i64_t.const_zero())
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        let mut call_args: Vec<inkwell::values::BasicMetadataValueEnum<'ctx>> =
            vec![argv_val.into()];
        call_args.extend(opt_vals);
        for slot in [pid_slot, in_slot, out_slot, err_slot] {
            call_args.push(slot.into());
        }
        if with_opts {
            call_args.push(deadline_slot.into());
        }
        let f = self
            .module
            .get_function(if with_opts {
                "lotus_process_spawn_with"
            } else {
                "lotus_process_spawn"
            })
            .expect("lotus_process_spawn[_with] declared");
        let ret_i32 = self
            .builder
            .build_call(
                f,
                &call_args,
                "proc.spawn.ret",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .try_as_basic_value()
            .left()
            .expect("returns i32")
            .into_int_value();
        let is_err = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::NE,
                ret_i32,
                i32_t.const_zero(),
                "proc.spawn.is_err",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        let info = self
            .user_types
            .get("__StdProcessSpawnHandle")
            .cloned()
            .expect("__StdProcessSpawnHandle declared in process.hl");
        let size = info
            .struct_ty
            .size_of()
            .expect("__StdProcessSpawnHandle has known size");
        let struct_ptr = self.arena_alloc(size, "SpawnHandle.alloc")?;

        let pid_i64 = self.load_i32_to_i64(pid_slot, "proc.spawn.pid.val")?;
        let in_i64 = self.load_i32_to_i64(in_slot, "proc.spawn.in.val")?;
        let out_i64 = self.load_i32_to_i64(out_slot, "proc.spawn.out.val")?;
        let err_i64 = self.load_i32_to_i64(err_slot, "proc.spawn.err.val")?;
        let deadline_i64 = self
            .builder
            .build_load(i64_t, deadline_slot, "proc.spawn.deadline.val")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        self.store_struct_field(
            &info, struct_ptr, "pid", pid_i64.into(), "SpawnHandle",
        )?;
        self.store_struct_field(
            &info, struct_ptr, "stdin_fd", in_i64.into(), "SpawnHandle",
        )?;
        self.store_struct_field(
            &info, struct_ptr, "stdout_fd", out_i64.into(), "SpawnHandle",
        )?;
        self.store_struct_field(
            &info, struct_ptr, "stderr_fd", err_i64.into(), "SpawnHandle",
        )?;
        self.store_struct_field(
            &info, struct_ptr, "deadline_ns", deadline_i64, "SpawnHandle",
        )?;

        // Same ENOENT space-in-argv[0] hint as run above.
        let path_label = self.process_surface_label_with_enoent_hint(
            argv_val,
            surface,
            &format!(
                "{} — argv[0] contains a space; argv is \
                 newline-separated (\"echo\\nhello\"), not shell-split",
                surface
            ),
            "proc.spawn",
        )?;
        self.complete_io_fallible_call(
            is_err,
            path_label.into(),
            Some((struct_ptr.into(), CodegenTy::TypeRef(
                "__StdProcessSpawnHandle".to_string()
            ))),
            "proc.spawn",
        )
    }

    /// The nine `SpawnOptions` arguments of `__run_with` /
    /// `__spawn_with`, in order: `cwd`, `env`, `unset_env` (String),
    /// `clear_env` (Bool), `timeout_ns`, `cpu_seconds`, `address_space`,
    /// `open_files`, `file_size` (Int) — lowered to the C side's
    /// `const char *` ×3 and `int64_t` ×6.
    fn lower_process_option_args(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
        surface: &str,
    ) -> Result<Vec<inkwell::values::BasicMetadataValueEnum<'ctx>>, CodegenError> {
        let i64_t = self.context.i64_type();
        let mut out = Vec::with_capacity(args.len());
        for (i, a) in args.iter().enumerate() {
            let (v, ty) = self.lower_expr(a, scope)?;
            match i {
                0..=2 => {
                    if !matches!(ty, CodegenTy::String | CodegenTy::StringView) {
                        return Err(CodegenError::Unsupported(format!(
                            "{}: option {} must be String, got {:?}",
                            surface, i, ty
                        )));
                    }
                    let v = self.unpack_view_if_needed(v, &ty)?;
                    out.push(v.into());
                }
                3 => {
                    if ty != CodegenTy::Bool {
                        return Err(CodegenError::Unsupported(format!(
                            "{}: clear_env must be Bool, got {:?}",
                            surface, ty
                        )));
                    }
                    let w = self
                        .builder
                        .build_int_cast_sign_flag(v.into_int_value(), i64_t, false, "proc.opt.clear")
                        .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
                    out.push(w.into());
                }
                _ => {
                    if !matches!(ty, CodegenTy::Int | CodegenTy::Duration) {
                        return Err(CodegenError::Unsupported(format!(
                            "{}: option {} must be Int, got {:?}",
                            surface, i, ty
                        )));
                    }
                    out.push(v.into_int_value().into());
                }
            }
        }
        Ok(out)
    }

    /// Store an `i1` into a `Bool` field of a result struct, cast to
    /// the field's own width.
    fn store_bool_field(
        &self,
        info: &TypeInfo<'ctx>,
        struct_ptr: PointerValue<'ctx>,
        field: &str,
        val: inkwell::values::IntValue<'ctx>,
        struct_label: &str,
    ) -> Result<(), CodegenError> {
        let (idx, _) = info
            .fields
            .get(field)
            .cloned()
            .unwrap_or_else(|| panic!("{}.{} field", struct_label, field));
        let field_ty = info
            .struct_ty
            .get_field_type_at_index(idx)
            .expect("field index in range")
            .into_int_type();
        let v = self
            .builder
            .build_int_cast_sign_flag(val, field_ty, false, &format!("{}.{}.cast", struct_label, field))
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.store_struct_field(info, struct_ptr, field, v.into(), struct_label)
    }
}

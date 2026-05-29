//! `std::time::*` path-call lowering (plus the pre-`std::*` aliases
//! `time::sleep` / `time::monotonic` from the m71/m79 era).

use hale_syntax::ast::Expr;
use inkwell::values::BasicValueEnum;

use crate::bus::runtime::BusRuntime;
use crate::codegen::{CodegenError, CodegenTy, Cx, Scope};

pub(crate) trait TimeStdlib<'ctx> {
    fn lower_time_monotonic_ns(
        &mut self,
        args: &[Expr],
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;

    fn lower_time_monotonic(
        &mut self,
        args: &[Expr],
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;

    fn lower_std_time_now(
        &mut self,
        args: &[Expr],
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;

    fn lower_std_time_from_unix(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError>;

    fn lower_time_sleep(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(), CodegenError>;
}

impl<'ctx, 'p> TimeStdlib<'ctx> for Cx<'ctx, 'p> {
    /// `std::time::monotonic_ns() -> Int` (2026-05-21). Same
    /// clock_gettime(CLOCK_MONOTONIC) shape as `monotonic()` but
    /// types the result as `Int` (i64 ns) instead of `Duration`.
    /// Kills the ASCII round-trip pattern downstream consumers
    /// were doing (`to_string(monotonic())` → strip "ns" →
    /// `parse_int`) at hot-path rates — three String ops per
    /// reading dropped to one syscall. The Duration return shape
    /// stays available for callers who actually want the
    /// formatted-Duration ergonomics.
    fn lower_time_monotonic_ns(
        &mut self,
        args: &[Expr],
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        let (value, _ty) = self.lower_time_monotonic(args)?;
        Ok((value, CodegenTy::Int))
    }

    /// Lower `time::monotonic()` to `clock_gettime(CLOCK_MONOTONIC,
    /// &ts)` followed by `ts.tv_sec * 1_000_000_000 + ts.tv_nsec`.
    /// Result is a `Duration` (i64 nanoseconds since an
    /// unspecified reference).
    fn lower_time_monotonic(
        &mut self,
        args: &[Expr],
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if !args.is_empty() {
            return Err(CodegenError::Unsupported(format!(
                "time::monotonic takes 0 arguments, got {}",
                args.len()
            )));
        }
        let i32_t = self.context.i32_type();
        let i64_t = self.context.i64_type();
        let ts_t = self.timespec_type();

        let ts = self.alloca_in_entry(ts_t.into(), "ts")?;
        let cgt = self
            .module
            .get_function("clock_gettime")
            .expect("clock_gettime declared");
        // CLOCK_MONOTONIC = 1 on Linux.
        let clock_id = i32_t.const_int(1, false);
        self.builder
            .build_call(cgt, &[clock_id.into(), ts.into()], "cgt.ret")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        // Ignore the return value best-effort; CLOCK_MONOTONIC
        // shouldn't fail. tv_sec * 1e9 + tv_nsec.
        let sec_ptr = self
            .builder
            .build_struct_gep(ts_t, ts, 0, "ts.sec.ptr")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let nsec_ptr = self
            .builder
            .build_struct_gep(ts_t, ts, 1, "ts.nsec.ptr")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let sec = self
            .builder
            .build_load(i64_t, sec_ptr, "ts.sec")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_int_value();
        let nsec = self
            .builder
            .build_load(i64_t, nsec_ptr, "ts.nsec")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_int_value();
        let billion = i64_t.const_int(1_000_000_000, false);
        let sec_ns = self
            .builder
            .build_int_mul(sec, billion, "sec.ns")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let total = self
            .builder
            .build_int_add(sec_ns, nsec, "now.ns")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        Ok((total.into(), CodegenTy::Duration))
    }

    /// C7 (pond follow-up): lower `std::time::now() -> Int` to a
    /// call into `lotus_time_now_seconds`, which wraps
    /// `clock_gettime(CLOCK_REALTIME, &ts)` and returns `ts.tv_sec`
    /// as i64. Wall-clock seconds since the Unix epoch — drives
    /// `pond/sessions` cookie expiries that must survive a
    /// process restart. Observation only; `time::monotonic` stays
    /// the basis for scheduling (NTP slewing / leap seconds can
    /// warp this value).
    fn lower_std_time_now(
        &mut self,
        args: &[Expr],
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if !args.is_empty() {
            return Err(CodegenError::Unsupported(format!(
                "std::time::now takes 0 arguments, got {}",
                args.len()
            )));
        }
        let now_fn = self
            .module
            .get_function("lotus_time_now_seconds")
            .expect("lotus_time_now_seconds declared");
        let call = self
            .builder
            .build_call(now_fn, &[], "time.now.sec")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let sec = call
            .try_as_basic_value()
            .left()
            .expect("lotus_time_now_seconds returns i64")
            .into_int_value();
        Ok((sec.into(), CodegenTy::Int))
    }

    /// `std::time::time_from_unix(n: Int) -> Time` — direct
    /// construction from epoch seconds. Lowers to a call into
    /// `lotus_time_from_unix`, which gmtime_r + strftime's the
    /// epoch into a 24-byte ISO 8601 UTC buffer in the caller arena.
    /// Mirrors the runtime shape of compile-time Time literals.
    fn lower_std_time_from_unix(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, CodegenTy), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "std::time::time_from_unix takes 1 arg (n), got {}",
                args.len()
            )));
        }
        let (n_val, n_ty) = self.lower_expr(&args[0], scope)?;
        if !matches!(n_ty, CodegenTy::Int) {
            return Err(CodegenError::Unsupported(format!(
                "std::time::time_from_unix: n must be Int, got {:?}",
                n_ty
            )));
        }
        let from_unix_fn = self
            .module
            .get_function("lotus_time_from_unix")
            .expect("lotus_time_from_unix declared");
        let call = self
            .builder
            .build_call(from_unix_fn, &[n_val.into()], "time.from_unix")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ptr = call
            .try_as_basic_value()
            .left()
            .expect("lotus_time_from_unix returns ptr");
        Ok((ptr, CodegenTy::Time))
    }

    /// Lower `time::sleep(duration)` to a monotonic-clock,
    /// EINTR-retrying `clock_nanosleep` call. The lowered IR is:
    ///
    /// ```text
    ///   sec = ns / 1_000_000_000
    ///   nsec = ns % 1_000_000_000
    ///   req.tv_sec  = sec
    ///   req.tv_nsec = nsec
    ///   while clock_nanosleep(CLOCK_MONOTONIC, 0, &req, &rem) == EINTR {
    ///       req = rem;   // resume from the remaining time
    ///   }
    /// ```
    ///
    /// `CLOCK_MONOTONIC` is hardcoded to 1 (Linux); flags = 0 means
    /// the request is relative (`TIMER_ABSTIME` would make it a
    /// deadline). Any non-EINTR error exits the loop best-effort —
    /// we don't crash the program over a clock failure.
    fn lower_time_sleep(
        &mut self,
        args: &[Expr],
        scope: &Scope<'ctx>,
    ) -> Result<(), CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::Unsupported(format!(
                "time::sleep takes 1 argument, got {}",
                args.len()
            )));
        }
        let (val, ty) = self.lower_expr(&args[0], scope)?;
        if ty != CodegenTy::Duration {
            return Err(CodegenError::Unsupported(format!(
                "time::sleep expects Duration, got {:?}",
                ty
            )));
        }
        let i32_t = self.context.i32_type();
        let i64_t = self.context.i64_type();
        let ts_t = self.timespec_type();
        let ns = val.into_int_value();
        let billion = i64_t.const_int(1_000_000_000, false);
        // 2026-05-29: chunk a long sleep into ≤100ms slices and drain
        // the cooperative bus queue after EACH slice. Previously the
        // drain happened only AFTER the whole sleep returned, so a
        // keep-alive `while true { sleep(60s); }` on the main thread
        // starved every main-pool bus handler (whose cells land on the
        // global cooperative queue that only main drains) for 60s at a
        // time — the dashboard's `on_data`-never-fires symptom. Slices
        // ≤100ms keep the queue serviced ~10×/s; short sleeps (≤100ms)
        // take exactly one slice, so the common case is unchanged.
        let chunk_max = i64_t.const_int(100_000_000, false); // 100ms
        let zero64 = i64_t.const_int(0, false);

        let req = self.alloca_in_entry(ts_t.into(), "req")?;
        let rem = self.alloca_in_entry(ts_t.into(), "rem")?;
        let remaining = self.alloca_in_entry(i64_t.into(), "sleep.remaining")?;
        let chunk_slot = self.alloca_in_entry(i64_t.into(), "sleep.chunk")?;
        self.builder
            .build_store(remaining, ns)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        let req_sec_ptr = self
            .builder
            .build_struct_gep(ts_t, req, 0, "req.sec.ptr")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let req_nsec_ptr = self
            .builder
            .build_struct_gep(ts_t, req, 1, "req.nsec.ptr")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        let func = self
            .current_fn
            .expect("current_fn set while lowering time::sleep");
        let chunk_bb = self.context.append_basic_block(func, "sleep.chunk");
        let loop_bb = self.context.append_basic_block(func, "sleep.loop");
        let retry_bb = self.context.append_basic_block(func, "sleep.retry");
        let drain_bb = self.context.append_basic_block(func, "sleep.drain");
        let done_bb = self.context.append_basic_block(func, "sleep.done");

        self.builder
            .build_unconditional_branch(chunk_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // chunk_bb (loop header): chunk = clamp(min(remaining, 100ms), 0);
        // store req from chunk; branch into the EINTR sleep loop.
        self.builder.position_at_end(chunk_bb);
        let rem_val = self
            .builder
            .build_load(i64_t, remaining, "sleep.rem.cur")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_int_value();
        let lt_max = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::SLT,
                rem_val,
                chunk_max,
                "sleep.lt.max",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let capped = self
            .builder
            .build_select(lt_max, rem_val, chunk_max, "sleep.capped")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_int_value();
        let neg = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::SLT,
                capped,
                zero64,
                "sleep.neg",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let chunk = self
            .builder
            .build_select(neg, zero64, capped, "sleep.chunk.val")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_int_value();
        self.builder
            .build_store(chunk_slot, chunk)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let sec = self
            .builder
            .build_int_signed_div(chunk, billion, "ts.sec")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let nsec = self
            .builder
            .build_int_signed_rem(chunk, billion, "ts.nsec")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_store(req_sec_ptr, sec)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_store(req_nsec_ptr, nsec)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_unconditional_branch(loop_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // loop_bb: call clock_nanosleep, branch on EINTR vs drain
        self.builder.position_at_end(loop_bb);
        let cns = self
            .module
            .get_function("clock_nanosleep")
            .expect("clock_nanosleep declared");
        // CLOCK_MONOTONIC = 1, flags = 0
        let clock_id = i32_t.const_int(1, false);
        let flags = i32_t.const_int(0, false);
        let call_result = self
            .builder
            .build_call(
                cns,
                &[
                    clock_id.into(),
                    flags.into(),
                    req.into(),
                    rem.into(),
                ],
                "cns.ret",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let ret_int = call_result
            .try_as_basic_value()
            .left()
            .expect("clock_nanosleep returns i32")
            .into_int_value();
        // EINTR == 4 on Linux. Everything else (including success=0)
        // exits the EINTR loop into the slice's drain.
        let eintr = i32_t.const_int(4, false);
        let is_eintr = self
            .builder
            .build_int_compare(inkwell::IntPredicate::EQ, ret_int, eintr, "is.eintr")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_conditional_branch(is_eintr, retry_bb, drain_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // retry_bb: copy rem → req, jump back into the loop
        self.builder.position_at_end(retry_bb);
        let rem_sec_ptr = self
            .builder
            .build_struct_gep(ts_t, rem, 0, "rem.sec.ptr")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let rem_nsec_ptr = self
            .builder
            .build_struct_gep(ts_t, rem, 1, "rem.nsec.ptr")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let rem_sec = self
            .builder
            .build_load(i64_t, rem_sec_ptr, "rem.sec")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let rem_nsec = self
            .builder
            .build_load(i64_t, rem_nsec_ptr, "rem.nsec")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_store(req_sec_ptr, rem_sec)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_store(req_nsec_ptr, rem_nsec)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_unconditional_branch(loop_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        // drain_bb: this slice elapsed. Drain the cooperative bus
        // queue so a long-running `while { time::sleep(...); ... }`
        // loop in a cooperative subscriber delivers cells posted by
        // other threads during the sleep (unix-bound reader threads,
        // pinned publishers, etc.) — without this the cells sat in
        // g_bus_queue until the body's next natural drain point (the
        // old workaround was `sleep; yield;`). 2026-05-23 mirror: if
        // this is a pinned locus's own thread, drain its mailbox too;
        // on a cooperative thread get_current is NULL → no-op. Then
        // subtract the slice from `remaining` and loop or finish.
        self.builder.position_at_end(drain_bb);
        self.emit_bus_drain()?;
        self.emit_pinned_mailbox_drain_pending()?;
        let chunk_done = self
            .builder
            .build_load(i64_t, chunk_slot, "sleep.chunk.done")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_int_value();
        let rem_after = self
            .builder
            .build_load(i64_t, remaining, "sleep.rem.after")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?
            .into_int_value();
        let new_rem = self
            .builder
            .build_int_sub(rem_after, chunk_done, "sleep.rem.next")
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_store(remaining, new_rem)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        let more = self
            .builder
            .build_int_compare(
                inkwell::IntPredicate::SGT,
                new_rem,
                zero64,
                "sleep.more",
            )
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;
        self.builder
            .build_conditional_branch(more, chunk_bb, done_bb)
            .map_err(|e| CodegenError::LlvmEmit(e.to_string()))?;

        self.builder.position_at_end(done_bb);
        Ok(())
    }
}

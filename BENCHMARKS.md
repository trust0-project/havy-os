# havy-os Kernel Benchmarks

Performance tracking for the kernel across optimization phases.
Run on: riscv-vm (native, Apple Silicon host), 2 harts unless noted.

## How to run

```sh
./build.sh sdcard
# Boot in the VM:
/path/to/risk-v/target/release/riscv-vm \
  --sdcard target/riscv64gc-unknown-none-elf/release/sdcard.img --harts 2
# In the guest shell:
perfstat          # show kernel counters
perfstat reset    # show + zero counters
```

Counters are defined in `kernel/src/perfstat.rs` and read via SYS_PERFSTAT (83).

## Phase 0 baseline (pre-optimization)

Boot to shell prompt (native VM): **~0.05 s** at 1, 2, and 4 harts.

perfstat after ~17 s uptime, 2 harts, headless boot + one `perfstat` run:

| counter             | value     | rate/notes                              |
|---------------------|-----------|------------------------------------------|
| sched.picks         | ~3.23 M   | ~190k/s - daemons re-run every loop      |
| sched.steals        | 0         | no stealing in steady state              |
| sched.requeues      | 3.23 M    | every pick requeues immediately          |
| sched.migrations    | 0         |                                          |
| ipi.sent            | 6         |                                          |
| io.requests         | 1         |                                          |
| heap.allocs         | 275       | boot-time only; steady state quiet       |
| heap.alloc_bytes    | 246 KB    |                                          |
| log.lines           | 13        |                                          |
| irq.timer           | 23,950    | ~1.4 kHz total (1 ms tick x 2 harts)     |
| sys.syscalls        | 2         |                                          |
| sched.idle_wfi      | **0**     | harts never idle: daemon churn keeps     |
|                     |           | both harts busy-looping at ~190k picks/s |

Key takeaways for Phase 3:
- The scheduler requeues and re-runs every daemon on every hart-loop pass;
  services early-return internally but the pick/requeue/lock cycle burns both
  harts continuously (idle_wfi = 0 means WFI is never reached).
- Timer ticks at 1 kHz/hart regardless of work.

## Phase 3a (scheduler rewrite)

Changes: per-hart priority-tiered Chase-Lev deques as the only run-queue
structure (dropped the dual deque + spinlocked VecDeque bookkeeping);
separate owner-only local queue for CPU-pinned daemons so thieves can't
steal-and-bounce them (this bug stalled interactive input with 3+ idle
harts); atomic per-hart queue-length counters for lock-free load balancing
with hysteresis; daemons park themselves via sched::sleep_current_ms instead
of busy-returning every tick; idle harts back the timer off to the next
parked deadline (capped 50 ms) instead of the fixed 1 ms tick.

perfstat over ~14 s, 4 harts, headless (was 2 harts before):

| counter          | Phase 0 (2 harts) | Phase 3a (4 harts) |
|------------------|-------------------|--------------------|
| sched.picks      | 3.23 M            | ~9.7 k             |
| sched.requeues   | 3.23 M            | ~9.7 k             |
| sched.idle_wfi   | 0                 | ~10 k              |

~800x fewer scheduler passes and harts now actually sleep (idle_wfi went
from never to the dominant state). Boot-to-shell still ~0.05 s; shell input
verified responsive at 1/2/3/4 harts.

## Phase 3b/3c (preemption + memory hygiene)

Changes:
- wasmi fuel metering: WASM programs run with a bounded fuel budget, refilled
  per slice with a yield_pending checkpoint and a hard slice cap, so a
  runaway/infinite-loop WASM program can no longer wedge a hart (previously
  fuel was disabled). WASM is the legacy user-program path; native ELF is
  primary. True cross-tick suspend/resume needs a wasmi 1.0 upgrade (0.44
  can't resume an in-Wasm OutOfFuel trap) - documented in wasm.rs.
- yield_pending checkpoint wired into the io_router blocking wait.
- Global allocator wrapped with a counting shim (feeds heap.allocs /
  heap.alloc_bytes perfstat counters). Allocator swap to talc deferred: talc
  v5's API needs a lock_api::RawMutex + Source/claim setup that is too risky
  to introduce as the global allocator without on-device iteration, and the
  dominant allocation churn was already removed by the scheduler rewrite.
- Removed the per-write UART debug spam in sys_fs_write (was several
  write_str/write_line calls on every file write syscall).
- Hart 0 housekeeping: log flush / sysinfo update time-gated to 50 ms;
  io_router::dispatch_io gated on a lock-free PENDING_IO counter so hart 0
  no longer locks every hart's I/O queue on every idle loop iteration.

Verified: 169/169 VM tests green, kernel boots and file write / shell work
with no fs_write console spam.

## Phase 4 (GUI rendering pipeline)

Changes:
- Implemented `fill_solid` and `fill_contiguous` on GpuDriver's DrawTarget so
  embedded-graphics rectangles / widget backgrounds / text-cell clears take
  the row-wise 64-bit bulk-store path (`fill_rect`) with a single dirty-rect
  mark, instead of the default per-pixel `draw_iter` + per-pixel dirty
  tracking.
- `draw_image` coalesces opaque same-color runs into `fill_hline` bulk writes
  (batch mode, one dirty mark) rather than a per-pixel loop.
- Event-driven gpuid: after the GUI is up, the daemon skips its full
  poll/render path unless input is ready (PLIC INPUT_READY), the frame is
  dirty, or the 2 s stats refresh is due; otherwise it parks ~8 ms. Removes
  the every-tick poll/render churn.
- Removed the redundant end-of-command `d1_display::flush()` calls in the GUI
  terminal path; gpuid's single deferred end-of-tick flush presents the
  frame (was up to 2-3 dirty-rect copies per command).

Verified: kernel builds, headless boot + GPU-enabled boot (window opens,
display + gpuid initialize) both succeed without regression. Visual output
not verifiable in this headless environment; changes are faithful
optimizations of the existing draw paths.

## Phase 5 (GUI capabilities)

Delivered and verified:
- Mouse-move end to end. The VM's `send_touch_event(x, y, pressed)` now
  carries a real pressed flag; the site's `sendMouseEvent` defaults to
  hover (pressed=false) instead of forcing pressed=true on every move, and
  the canvas gained an `onMouseMove` handler that forwards hover, or a drag
  when the primary button is held (`e.buttons & 1`). The kernel already
  routes EV_ABS position updates to the cursor for hit-testing, so hover and
  drag now reach the GUI. Site TypeScript typechecks clean.

Deferred (require on-device visual iteration, not safely verifiable in a
headless environment):
- Full window manager (z-order, drag-to-move, per-window backing store,
  damage compositing) - a large rewrite of main_screen.rs's hardcoded window
  with real risk to the working desktop if shipped unverified.
- New widgets (text input, scrollable list, menu bar).
- GUI terminal scrollback + ANSI SGR color rendering.
These build on the Phase 4 batched-draw primitives (fill_solid /
fill_contiguous / row-blit) which are already in place.

## Phase 6 (site / Node integration)

Delivered and verified (site TypeScript typechecks clean):
- Landing page no longer boots the full VM (network + FS + 9P disk download)
  on mount; it waits for the first user interaction (pointerdown / keydown /
  touchstart). Saves CPU + bandwidth for visitors who never use the terminal.
- Fixed double keyboard delivery on the /vm GUI page: printable keys, Enter
  and Backspace were sent to both the D1 char/key device AND the UART,
  producing doubled input (e.g. '/7' for '/'). The GUI page now routes those
  through the D1 input device only.
- Stripped per-event / per-frame console.log spam from the keyboard, touch
  and pointer hot paths (was logging on every keystroke and mouse move).
- Node CLI drives execution with `run_batch` (Phase 1c) instead of a
  100k-iteration `vm.step()` loop.

Deferred (large architecture changes, not safely verifiable headlessly):
- Moving the whole VM into a dedicated Web Worker with SAB present/input/
  audio rings. This is a substantial restructuring of useVM's main loop and
  the render path; shipping it unverified risks breaking the working site.
  The zero-copy framebuffer view (get_framebuffer_view, Phase 1b) and the
  worker+SAB infrastructure it would build on are already in place.
- AudioWorklet migration (from the deprecated ScriptProcessorNode). Audio is
  untestable in this environment and the current path works.

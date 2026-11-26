# Sprint 14a — frame-rate and timer-step unlock

Ported from Belobug/VIctory-Belobug-AMD commit 722c0e1ce1e80bd045ba63ecb88c440041233fea (including 49cb83b and 08b88b8).
The RX6800 launcher defaults to 16.667 ms (60 FPS), with timer logging; the experimental FSR path remains unchanged unless explicitly overridden. Other launchers are unchanged. The retail hash and all three original instruction windows must match; any failed site prevents title execution.

`VICTORY_FRAME_INTERVAL_MS` changes the compatibility layer's frame-event clock. `VICTORY_TITLE_FPS` supplies the title's two FPS loads and minimum timer step. `VICTORY_TIMER_LOG=1` logs frame beginnings, timer values and pacing bands. The maximum timer step (documented upstream as 1/15 s) remains unchanged. This is not a simulation/render-thread decoupling or an FPS multiplier.

Use the normal Series S command, or override both interval and title rate for experiments. Restore stock timing with `-More @{VICTORY_FRAME_INTERVAL_MS='';VICTORY_TIMER_LOG='0';VICTORY_RENDER_SCALE='0.30'}`. Timer reads are enabled only with a verified timing session. No DLL or shader cache deletion is required.

Windows build and gameplay validation pending. Check stderr for 3/3 reads patched, 16.667 ms interval, and timer steps. Compare timer advancement over several wall-clock seconds at low native FPS; menu timers and paused game timers are not evidence of gameplay simulation speed.

# Sprint 13a: PC FidelityFX 4.1.2 bridge

Ported from Belobug/VIctory-Belobug-AMD commit cd8dfc06ace7ccb91a19431e74bc2d0de2a10145 (DexNom ab34a8b2).
Normal launcher behavior is unchanged: Series S, title's own upscaler, Relay 0, CBV disabled.

Obtain the official AMD FidelityFX SDK 2.4.0 signed DX12 upscaler DLL and set VICTORY_FIDELITYFX_DIR to its folder (or paths.fidelityfx_dir in victory.toml). Run:

```powershell
.\scripts\play-rx6800.ps1 -Exe "F:\Games\GTAVI\game_scarlett_master.exe" -Fsr4K performance -CoolFoxResidency
```

The experimental profile uses the stock Series X 4K SDR frontend and 1920x1080 input (quality 2560x1440, balanced 2258x1270, ultra-performance / 720p 1280x720). The executable hash and instruction bytes are verified before patching. The PC provider is queried on the actual host device and must advertise 4.1.2; missing/unsupported providers fail with an error. Existing Series S scale hooks, texture submission prelude and asynchronous submission are disabled for this opt-in launcher mode. Separate image caches per preset. No DLL is bundled.

This is not a claim that RX 6800 supports FSR 4.1.2. RDNA2-compatible provider selection and frame generation are separate tasks. Windows compilation and runtime validation are required.

For 720p input upscaled to 4K, use `-Fsr4K ultra-performance` (alias `-Fsr4K 720p`). This is the SDK's standard 3x per-axis mode, not a custom render scale. The actual dispatch dimensions must still be verified in the runtime log. Other FSR compatibility restrictions remain unchanged.

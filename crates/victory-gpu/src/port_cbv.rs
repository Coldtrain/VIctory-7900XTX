//! Renderer port 5: direct constant-buffer binding and optional raw table
//! translation. Aligned zero-copy works with the original shader translation;
//! raw table lowering is a separate experiment. Ordinary title heap slots
//! and root CBVs keep their original descriptor conventions.
use std::sync::{OnceLock, atomic::{AtomicU64, Ordering::Relaxed}};

pub(crate) fn on() -> bool {
    victory_shader::emit::constants_raw()
}

/// Direct aligned CBVs, independent of the optional raw shader lowering.
/// Only under synchronous Relay 0; this preserves the title's ordering
/// assumptions about the lifetime of ring constants.
pub(crate) fn in_place_on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        let requested = std::env::var_os("VICTORY_PORT_CBV_PLACE").is_some_and(|v| v == "1");
        let enabled = requested && !crate::host::late() && !crate::relay_next::enabled();
        tracing::info!(enabled, raw_tables = on(), "renderer port 5: aligned zero-copy CBV placement");
        enabled
    })
}

static STATS: [AtomicU64; 6] = [const { AtomicU64::new(0) }; 6];

pub(crate) fn in_place(size: u32) {
    STATS[0].fetch_add(1, Relaxed);
    STATS[1].fetch_add(u64::from(size), Relaxed);
}

pub(crate) fn copied(not_in_place: bool) {
    STATS[if not_in_place { 2 } else { 3 }].fetch_add(1, Relaxed);
}

pub(crate) fn scratch_copy(size: u32) {
    if in_place_on() || on() {
        STATS[4].fetch_add(1, Relaxed);
        STATS[5].fetch_add(u64::from(size), Relaxed);
    }
}

pub(crate) fn raw_ok(gpu: u64) -> bool {
    gpu.is_multiple_of(16)
}

pub(crate) fn log(frames: f64) {
    let placement = in_place_on();
    let raw = on();
    if !placement && !raw {
        return;
    }
    let s = STATS.each_ref().map(|v| v.swap(0, Relaxed) as f64 / frames.max(1e-9));
    tracing::info!(
        placement, raw_tables = raw,
        "renderer port 5 per frame: {:.0} tables' constant buffers read by the card in place ({:.1} KB not copied, no step at submission); {:.0} scratch copies ({:.1} KB copied); rejected placement: {:.0} not in place/full span, {:.0} unaligned or invalid",
        s[0], s[1] / 1024.0, s[4], s[5] / 1024.0, s[2], s[3],
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_views_need_sixteen_bytes() {
        assert!(raw_ok(0x3500_0010));
        assert!(raw_ok(0x3500_0100));
        assert!(!raw_ok(0x3500_0008));
        assert!(!raw_ok(0x3500_0004));
    }
}

//! Relay Next, sprint R6: GPU-resident merge results and hash-only refreshes.
//! Independent of the old relay flight queue. Preparation and title-memory
//! merging deliberately retain the working synchronous implementation.

use std::sync::{Arc, OnceLock};
use std::sync::atomic::{AtomicU64, Ordering};
use std::collections::HashSet;
use windows::core::{Interface, IUnknown};
use windows::Win32::Foundation::{HANDLE, WAIT_OBJECT_0};
use windows::Win32::Graphics::Direct3D12::*;
use windows::Win32::System::Threading::{INFINITE, WaitForSingleObject};

fn mode() -> u8 {
    static MODE: OnceLock<u8> = OnceLock::new();
    *MODE.get_or_init(|| match std::env::var("VICTORY_RELAY_NEXT").as_deref() {
        Err(_) | Ok("0") => 0,
        Ok("1") => 1,
        Ok("2") => 2,
        Ok(mode) => panic!("VICTORY_RELAY_NEXT={mode} is not implemented; use 0, 1 or 2"),
    })
}

pub(crate) fn enabled() -> bool { mode() != 0 }

/// R3 permits at most two tracked GPU batches in a single prepared ticket.
/// Independent title submissions still complete in order.
pub(crate) fn paired() -> bool { mode() == 2 }

/// R9a uses the existing ordered queue lane without enabling legacy Relay.
/// The lane owns detached recordings; each ticket still lands in order.
pub(crate) fn scheduler() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| paired()
        && std::env::var_os("VICTORY_RELAY_NEXT_SCHEDULER").is_some_and(|v| v == "1")
        && std::env::var_os("VICTORY_GPU_TIMING").is_none())
}

/// Bound how much work is queued ahead of the GPU owner.
pub(crate) fn scheduler_depth() -> usize {
    static N: OnceLock<usize> = OnceLock::new();
    *N.get_or_init(|| std::env::var("VICTORY_RELAY_NEXT_QUEUE_DEPTH").ok()
        .and_then(|v| v.parse::<usize>().ok()).unwrap_or(4).clamp(1, 8))
}

/// R9b broadens admission without lifting the dependency checks.
pub(crate) fn dependency_batches() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| scheduler() && std::env::var_os("VICTORY_RELAY_NEXT_DEPENDENCY_BATCHES").is_some_and(|v| v == "1"))
}

/// R9c permits adjacent ready work from different title queues in one
/// ordered ticket. It never bypasses an unresolved wait or signal.
pub(crate) fn cross_queue_batches() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| dependency_batches() && std::env::var_os("VICTORY_RELAY_NEXT_CROSS_QUEUE").is_some_and(|v| v == "1"))
}

// Candidate selection counters, not just the relatively rare attempts at
// the range-admission check. Report even when no candidate is admitted.
static LANE_COUNTS: [AtomicU64; 6] = [const { AtomicU64::new(0) }; 6];
pub(crate) fn lane_candidate(reason: usize) {
    LANE_COUNTS[reason.min(5)].fetch_add(1, Ordering::Relaxed);
}
fn lane_report(calls: u64) {
    tracing::info!(calls,
        empty = LANE_COUNTS[0].load(Ordering::Relaxed),
        earlier = LANE_COUNTS[1].load(Ordering::Relaxed),
        candidate = LANE_COUNTS[2].load(Ordering::Relaxed),
        boundary = LANE_COUNTS[3].load(Ordering::Relaxed),
        cross_joined = LANE_COUNTS[4].load(Ordering::Relaxed),
        satisfied_wait = LANE_COUNTS[5].load(Ordering::Relaxed),
        "Relay Next R9c cross-queue scheduler statistics");
}

/// A final compare batch may follow the title parts: seven parts means
/// at most eight GPU batches, all with their original queue boundaries.
pub(crate) fn scheduler_parts() -> usize {
    static N: OnceLock<usize> = OnceLock::new();
    *N.get_or_init(|| std::env::var("VICTORY_RELAY_NEXT_JOIN_PARTS").ok()
        .and_then(|v| v.parse::<usize>().ok()).unwrap_or(4)
        .clamp(1, if dependency_batches() { 7 } else { 4 }))
}

pub(crate) fn join_decision(rejection: Option<usize>) {
    static CALLS: AtomicU64 = AtomicU64::new(0);
    static ACCEPTED: AtomicU64 = AtomicU64::new(0);
    static REJECTED: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
    let calls = CALLS.fetch_add(1, Ordering::Relaxed) + 1;
    if let Some(reason) = rejection {
        REJECTED[reason.min(3)].fetch_add(1, Ordering::Relaxed);
    } else { ACCEPTED.fetch_add(1, Ordering::Relaxed); }
    if calls == 1 || calls.is_multiple_of(256) || trace() {
        tracing::info!(calls, accepted = ACCEPTED.load(Ordering::Relaxed),
            visibility = REJECTED[0].load(Ordering::Relaxed),
            opaque = REJECTED[1].load(Ordering::Relaxed),
            overflow = REJECTED[2].load(Ordering::Relaxed),
            alias = REJECTED[3].load(Ordering::Relaxed),
            "Relay Next R9b dependency admission statistics");
    }
}

pub(crate) fn scheduler_stats(parts: usize, lists: usize, elapsed_us: u64) {
    static CALLS: AtomicU64 = AtomicU64::new(0);
    static JOINED: AtomicU64 = AtomicU64::new(0);
    let calls = CALLS.fetch_add(1, Ordering::Relaxed) + 1;
    JOINED.fetch_add(parts.saturating_sub(1) as u64, Ordering::Relaxed);
    if calls == 1 || calls.is_multiple_of(256) || trace() {
        tracing::info!(calls, parts, lists, elapsed_us, joined = JOINED.load(Ordering::Relaxed),
            "Relay Next R9a independent scheduler statistics");
        if cross_queue_batches() { lane_report(calls); }
    }
}

/// R10 hashes exact sparse dirty-piece indices using one dispatch per
/// buffer (or a bounded sequence for very large buffers).
pub(crate) fn sparse_hash() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| paired() && std::env::var_os("VICTORY_RELAY_NEXT_SPARSE_HASH").is_some_and(|v| v == "1"))
}

pub(crate) fn sparse_hash_stats(old: u64, new: u64, pieces: u64) {
    static CALLS: AtomicU64 = AtomicU64::new(0);
    static OLD: AtomicU64 = AtomicU64::new(0);
    static NEW: AtomicU64 = AtomicU64::new(0);
    static PIECES: AtomicU64 = AtomicU64::new(0);
    let calls = CALLS.fetch_add(1, Ordering::Relaxed) + 1;
    let old = OLD.fetch_add(old, Ordering::Relaxed) + old;
    let new = NEW.fetch_add(new, Ordering::Relaxed) + new;
    let pieces = PIECES.fetch_add(pieces, Ordering::Relaxed) + pieces;
    if calls == 1 || calls.is_multiple_of(256) || trace() {
        tracing::info!(calls, old_dispatches = old, new_dispatches = new, saved_dispatches = old.saturating_sub(new), pieces,
            "Relay Next R10 sparse hash statistics");
    }
}

pub(crate) fn snapshots() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| paired() && std::env::var_os("VICTORY_RELAY_NEXT_SNAPSHOTS").is_none_or(|v| v != "0"))
}

pub(crate) fn parallel_landing() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| snapshots() && std::env::var_os("VICTORY_RELAY_NEXT_MERGE").is_some_and(|v| v == "1"))
}

pub(crate) fn resident_rehash() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| paired() && std::env::var_os("VICTORY_RELAY_NEXT_RESIDENT").is_none_or(|v| v != "0"))
}

/// R6b can be disabled independently of the stable R6 rehash path.
pub(crate) fn resident_merge() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| resident_rehash() && std::env::var_os("VICTORY_RELAY_NEXT_RESIDENT_MERGE").is_none_or(|v| v != "0"))
}

/// R7a: finer GPU comparison grid is optional until measured on hardware.
pub(crate) fn readback_piece_bytes() -> usize {
    static SIZE: OnceLock<usize> = OnceLock::new();
    *SIZE.get_or_init(|| {
        match std::env::var("VICTORY_RELAY_NEXT_READBACK_KB").as_deref() {
            Ok("16") => 16 << 10,
            Ok("32") => 32 << 10,
            _ => 64 << 10,
        }
    })
}

pub(crate) fn fine_readback_on() -> bool {
    crate::host::next_card_piece_bytes() < 65536
}

/// Export changed bytes in the compare pass; keep R7a as an independent
/// fallback when disabled or when the bounded arena fills.
pub(crate) fn packed_readback() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| snapshots() && std::env::var_os("VICTORY_RELAY_NEXT_PACKED_READBACK").is_some_and(|v| v == "1"))
}

pub(crate) fn packed_stats(hit: bool, pieces: usize, bytes: u64) {
    static CALLS: AtomicU64 = AtomicU64::new(0);
    static AVOIDED: AtomicU64 = AtomicU64::new(0);
    static FALLBACK: AtomicU64 = AtomicU64::new(0);
    static PACKED_BYTES: AtomicU64 = AtomicU64::new(0);
    let calls = CALLS.fetch_add(1, Ordering::Relaxed) + 1;
    if hit {
        AVOIDED.fetch_add(1, Ordering::Relaxed);
        PACKED_BYTES.fetch_add(bytes, Ordering::Relaxed);
    } else { FALLBACK.fetch_add(1, Ordering::Relaxed); }
    if calls == 1 || calls.is_multiple_of(128) || trace() {
        tracing::info!(calls, pieces, bytes, hit,
            waits_avoided = AVOIDED.load(Ordering::Relaxed),
            fallback = FALLBACK.load(Ordering::Relaxed),
            packed_bytes = PACKED_BYTES.load(Ordering::Relaxed),
            "Relay Next R7b packed readback statistics");
    }
}

pub(crate) fn readback_stats(pieces: usize, bytes: u64, elapsed_us: u64) {
    static CALLS: AtomicU64 = AtomicU64::new(0);
    static BYTES: AtomicU64 = AtomicU64::new(0);
    static US: AtomicU64 = AtomicU64::new(0);
    let calls = CALLS.fetch_add(1, Ordering::Relaxed) + 1;
    BYTES.fetch_add(bytes, Ordering::Relaxed);
    US.fetch_add(elapsed_us, Ordering::Relaxed);
    if calls == 1 || calls.is_multiple_of(256) || trace() {
        tracing::info!(calls, pieces, bytes, elapsed_us,
            total_bytes = BYTES.load(Ordering::Relaxed),
            total_us = US.load(Ordering::Relaxed), piece_bytes = readback_piece_bytes(),
            "Relay Next R7a fine readback statistics");
    }
}

/// Counts bytes not queued for the redundant merge-result upload. Existing
/// dirty ranges may still overlap; this is not a hardware bandwidth counter.
pub(crate) fn resident_stats(bytes: u64, kept: bool) {
    static CALLS: AtomicU64 = AtomicU64::new(0);
    static KEPT: AtomicU64 = AtomicU64::new(0);
    static UPLOAD: AtomicU64 = AtomicU64::new(0);
    let count = CALLS.fetch_add(1, Ordering::Relaxed) + 1;
    if kept { KEPT.fetch_add(bytes, Ordering::Relaxed); }
    else { UPLOAD.fetch_add(bytes, Ordering::Relaxed); }
    if count == 1 || count.is_multiple_of(4096) || trace() {
        tracing::info!(count, hash_only_bytes = KEPT.load(Ordering::Relaxed),
            upload_bytes = UPLOAD.load(Ordering::Relaxed), "Relay Next R6 resident merge statistics");
    }
}

pub(crate) fn merge_workers() -> usize {
    static N: OnceLock<usize> = OnceLock::new();
    *N.get_or_init(|| std::env::var("VICTORY_RELAY_NEXT_MERGE_WORKERS").ok()
        .and_then(|v| v.parse::<usize>().ok()).unwrap_or(4).clamp(1, 8))
}

/// Conservative page-granularity check. Different jobs must never write
/// the same title page, even through distinct shadow copies.
pub(crate) fn disjoint_pages(mut ranges: Vec<(usize, usize)>) -> bool {
    if ranges.iter().any(|&(lo, hi)| lo >= hi) { return false; }
    ranges.sort_unstable();
    ranges.windows(2).all(|w| (w[0].1 - 1) / 4096 < w[1].0 / 4096)
}

type ScanWork = Vec<(Arc<crate::shadow::Shadow>, usize, usize)>;
type ScanResult = Vec<Vec<(usize, usize)>>;
type ScanJob = (ScanWork, std::sync::mpsc::SyncSender<ScanResult>);

/// Read-only comparison on a persistent worker while the submitting thread
/// fetches the carded copies. No title writes are moved into this worker.
pub(crate) fn start_scan(work: ScanWork) -> Option<std::sync::mpsc::Receiver<ScanResult>> {
    if !parallel_landing() || work.is_empty() { return None; }
    static WORKER: OnceLock<Option<std::sync::mpsc::SyncSender<ScanJob>>> = OnceLock::new();
    let worker = WORKER.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::sync_channel::<ScanJob>(1);
        match std::thread::Builder::new().name("victory-next-scan".into()).spawn(move || {
            crate::cores::keep_helper_on_fast_cores("victory-next-scan");
            while let Ok((work, done)) = rx.recv() {
                let found = crate::shadow::find_all(&work);
                if done.send(found).is_err() { break; }
            }
        }) {
            Ok(_) => Some(tx),
            Err(e) => {
                tracing::warn!(error = %e, "Relay Next R5 scan worker unavailable; using inline scan");
                None
            }
        }
    }).as_ref()?;
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    worker.send((work, tx)).unwrap_or_else(|_| crate::host::fail(format_args!("Relay Next scan worker disconnected")));
    Some(rx)
}

pub(crate) fn landing_stats(pieces: usize, bytes: u64, parallel: bool, elapsed_us: u64) {
    static CALLS: AtomicU64 = AtomicU64::new(0);
    static PARALLEL: AtomicU64 = AtomicU64::new(0);
    static BYTES: AtomicU64 = AtomicU64::new(0);
    static US: AtomicU64 = AtomicU64::new(0);
    let calls = CALLS.fetch_add(1, Ordering::Relaxed) + 1;
    let parallel_calls = PARALLEL.fetch_add(u64::from(parallel), Ordering::Relaxed) + u64::from(parallel);
    let total_bytes = BYTES.fetch_add(bytes, Ordering::Relaxed) + bytes;
    let total_us = US.fetch_add(elapsed_us, Ordering::Relaxed) + elapsed_us;
    if calls == 1 || calls.is_multiple_of(256) || trace() {
        tracing::info!(calls, parallel_calls, pieces, bytes, elapsed_us, total_bytes, total_us,
            workers = merge_workers(), "Relay Next R5 landing statistics");
    }
}

fn trace() -> bool {
    static TRACE: OnceLock<bool> = OnceLock::new();
    *TRACE.get_or_init(|| std::env::var_os("VICTORY_RELAY_NEXT_TRACE").is_some_and(|v| v == "1"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Preparing,
    InFlight(u64),
    GpuComplete(u64),
    CpuVisible,
}

#[derive(Debug)]
struct Progress {
    phase: Phase,
    pending: usize,
    peak: usize,
}

impl Progress {
    fn new() -> Self {
        Self { phase: Phase::Preparing, pending: 0, peak: 0 }
    }

    fn sent(&mut self, fence: u64) -> bool {
        self.enqueue(fence, 1)
    }

    fn enqueue(&mut self, fence: u64, limit: usize) -> bool {
        self.enqueue_bounded(fence, limit.min(2))
    }

    fn enqueue_bounded(&mut self, fence: u64, limit: usize) -> bool {
        if fence == 0 || fence == u64::MAX || self.pending >= limit.min(8) || !match self.phase {
            Phase::Preparing => true,
            Phase::GpuComplete(last) | Phase::InFlight(last) => fence > last,
            Phase::CpuVisible => false,
        } {
            return false;
        }
        self.pending += 1;
        self.peak = self.peak.max(self.pending);
        self.phase = Phase::InFlight(fence);
        true
    }

    fn completed(&mut self, actual: u64) -> bool {
        let Phase::InFlight(target) = self.phase else { return false };
        if actual == u64::MAX || actual < target {
            return false;
        }
        self.pending = 0;
        self.phase = Phase::GpuComplete(target);
        true
    }

    fn visible(&mut self) -> bool {
        if self.pending != 0 || !matches!(self.phase, Phase::GpuComplete(_)) {
            return false;
        }
        self.phase = Phase::CpuVisible;
        true
    }
}

/// One recording remains owned by its submission until CPU-visible completion.
#[derive(Default, Debug)]
pub(crate) struct RecordingLease(AtomicU64);

impl RecordingLease {
    fn claim(&self, number: u64) -> bool {
        number != 0 && self.0.compare_exchange(0, number, Ordering::AcqRel, Ordering::Acquire).is_ok()
    }

    fn release(&self, number: u64) -> bool {
        self.0.compare_exchange(number, 0, Ordering::AcqRel, Ordering::Acquire).is_ok()
    }

    pub(crate) fn assert_idle(&self) {
        let owner = self.0.load(Ordering::Acquire);
        if owner != 0 {
            crate::host::fail(format_args!("Relay Next recording reset before CPU-visible completion: owner={owner}"));
        }
    }
}

/// A ticket spans the existing submission lock, including CPU copy-back.
/// R3 can queue two GPU batches, but never overlaps two title tickets.
pub(crate) struct Ticket {
    number: u64,
    progress: Progress,
    groups: usize,
    start: std::time::Instant,
    leases: Vec<Arc<RecordingLease>>,
    lease_keys: HashSet<usize>,
    pins: Vec<IUnknown>,
    pin_keys: HashSet<usize>,
}

impl Ticket {
    pub(crate) fn new(number: u64) -> Self {
        static START: std::sync::Once = std::sync::Once::new();
        START.call_once(|| tracing::info!(
            mode = mode(), max_batches = if scheduler() { scheduler_parts().max(2) + usize::from(scheduler_parts() >= 2) } else if paired() { 2 } else { 1 }, max_title_tickets = 1, copy_queue = false,
            independent_scheduler = scheduler(), queue_depth = scheduler_depth(), dependency_batches = dependency_batches(),
            private_snapshots = snapshots(), parallel_landing = parallel_landing(), merge_workers = merge_workers(),
            resident_rehash = crate::host::next_resident_rehash(), resident_merge = resident_merge(),
            "Relay Next R6: resident merge results; legacy Relay disabled"
        ));
        Self { number, progress: Progress::new(), groups: 0, start: std::time::Instant::now(),
            leases: Vec::new(), lease_keys: HashSet::new(), pins: Vec::new(), pin_keys: HashSet::new() }
    }

    pub(crate) fn claim(&mut self, lease: &Arc<RecordingLease>) {
        if !self.lease_keys.insert(Arc::as_ptr(lease) as usize) { return; }
        if !lease.claim(self.number) {
            crate::host::fail(format_args!("Relay Next recording already owned: submission {}", self.number));
        }
        self.leases.push(lease.clone());
    }

    pub(crate) fn pin(&mut self, object: &impl Interface) {
        if !self.pin_keys.insert(object.as_raw() as usize) { return; }
        let keep = object.cast::<IUnknown>().unwrap_or_else(|e| {
            crate::host::fail(format_args!("Relay Next could not retain submission {} resource: {e}", self.number))
        });
        self.pins.push(keep);
    }

    pub(crate) fn send(
        &mut self, device: &ID3D12Device, queue: &ID3D12CommandQueue,
        fence: &ID3D12Fence, target: u64,
        lists: &[Option<ID3D12CommandList>], overlap: bool, batch_limit: usize, queued: impl FnOnce(),
    ) {
        let accepted = if overlap && paired() && scheduler() && batch_limit > 2 {
            self.progress.enqueue_bounded(target, batch_limit)
        } else if overlap && paired() { self.progress.enqueue(target, 2) } else { self.progress.sent(target) };
        if !accepted {
            crate::host::fail(format_args!("Relay Next invalid submission transition: {} {:?}", self.number, self.progress));
        }
        for list in lists.iter().flatten() { self.pin(list); }
        // SAFETY: the caller holds the submission lock; lists and their
        // backing allocations remain owned until synchronous completion.
        unsafe {
            device.GetDeviceRemovedReason().unwrap_or_else(|e| {
                crate::host::fail(format_args!("Relay Next device removed before submit {}: {e}", self.number))
            });
            queue.ExecuteCommandLists(lists);
            queued();
            queue.Signal(fence, target)
                .unwrap_or_else(|e| crate::host::fail(format_args!("Relay Next signal failed for {} fence {target}: {e}", self.number)));
        }
        self.groups += 1;
        if self.progress.pending > 2 && trace() {
            tracing::info!(submission = self.number, target, pending = self.progress.pending,
                "Relay Next R9b multiple title batches in flight");
        }
        if self.progress.pending == 2 {
            static PAIRS: AtomicU64 = AtomicU64::new(0);
            let pairs = PAIRS.fetch_add(1, Ordering::Relaxed) + 1;
            if pairs == 1 || pairs.is_multiple_of(256) || trace() {
                tracing::info!(submission = self.number, target, pairs, "Relay Next R3: two GPU batches queued without an intermediate CPU wait");
            }
        }
        if trace() {
            tracing::info!(submission = self.number, target, lists = lists.len(), "Relay Next submitted");
        }
    }

    pub(crate) fn wait(&mut self, device: &ID3D12Device, fence: &ID3D12Fence, event: HANDLE) {
        let Phase::InFlight(target) = self.progress.phase else {
            crate::host::fail(format_args!("Relay Next wait without pending GPU work: submission {}", self.number));
        };
        // Arm only the highest fence, once. Registering the same auto-reset
        // event for both batches could wake on the first fence prematurely.
        unsafe {
            fence.SetEventOnCompletion(target, event).unwrap_or_else(|e|
                crate::host::fail(format_args!("Relay Next fence event registration failed for {target}: {e}")));
        }
        let status = unsafe { WaitForSingleObject(event, INFINITE) };
        let completed = unsafe { fence.GetCompletedValue() };
        let health = unsafe { device.GetDeviceRemovedReason() };
        if status != WAIT_OBJECT_0 || health.is_err() || !self.progress.completed(completed) {
            crate::host::fail(format_args!("Relay Next completion failed: submission {} phase {:?} fence {completed} wait {status:?} device {health:?}", self.number, self.progress));
        }
        if trace() {
            tracing::info!(submission = self.number, completed, "Relay Next GPU complete");
        }
    }

    pub(crate) fn check_device(&self, device: &ID3D12Device) {
        if let Err(e) = unsafe { device.GetDeviceRemovedReason() } {
            crate::host::fail(format_args!("Relay Next submission {} failed before CPU-visible completion: {e}", self.number));
        }
    }

    pub(crate) fn finish(mut self, device: &ID3D12Device) {
        self.check_device(device);
        if !self.progress.visible() {
            crate::host::fail(format_args!("Relay Next premature CPU-visible completion: {}", self.number));
        }
        if trace() || self.number.is_multiple_of(256) {
            tracing::info!(submission = self.number, groups = self.groups,
                recordings = self.leases.len(), pinned_objects = self.pins.len(), peak_batches = self.progress.peak,
                elapsed_us = self.start.elapsed().as_micros() as u64, "Relay Next CPU visible");
        }
        for lease in &self.leases {
            if !lease.release(self.number) {
                crate::host::fail(format_args!("Relay Next recording ownership lost: submission {}", self.number));
            }
        }
        // All explicit COM pins are released only after the visibility check.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_overlapping_and_premature_completion() {
        let mut p = Progress::new();
        assert!(!p.visible());
        assert!(p.sent(4));
        assert!(!p.sent(5));
        assert!(!p.completed(3));
        assert!(!p.completed(u64::MAX));
        assert!(!p.visible());
        assert!(p.completed(4));
        assert!(!p.sent(4));
        assert!(p.sent(6));
        assert!(p.completed(6));
        assert!(p.visible());
        assert!(!p.sent(7));
        assert!(!p.visible());
    }
    #[test]
    fn parallel_landing_rejects_shared_pages_and_overlaps() {
        assert!(disjoint_pages(vec![(8192, 12288), (0, 4096)]));
        assert!(disjoint_pages(vec![(0, 4096), (4096, 8192)]));
        assert!(!disjoint_pages(vec![(0, 2048), (2048, 4096)]));
        assert!(!disjoint_pages(vec![(4096, 8192), (4096, 8192)]));
        assert!(!disjoint_pages(vec![(0, 8192), (4096, 12288)]));
        assert!(!disjoint_pages(vec![(1, 1)]));
    }

    #[test]
    fn two_batches_require_last_fence_and_reject_third() {
        let mut p = Progress::new();
        assert!(p.enqueue(10, 2));
        assert!(p.enqueue(11, 2));
        assert_eq!(p.peak, 2);
        assert!(!p.enqueue(12, 2));
        assert!(!p.enqueue(12, usize::MAX));
        assert!(!p.completed(10));
        assert!(!p.completed(u64::MAX));
        assert!(!p.visible());
        assert!(p.completed(11));
        assert_eq!(p.pending, 0);
        assert!(p.visible());
    }

    #[test]
    fn scheduler_eight_batches_need_final_fence_and_reject_ninth() {
        let mut p = Progress::new();
        for f in 11..=18 { assert!(p.enqueue_bounded(f, 8)); }
        assert_eq!(p.peak, 8);
        assert!(!p.enqueue_bounded(19, 9));
        assert!(!p.completed(17));
        assert!(!p.visible());
        assert!(p.completed(18));
        assert!(p.visible());
    }
    #[test]
    fn scheduler_five_batches_need_final_fence_and_reject_sixth() {
        let mut p = Progress::new();
        for f in 11..=15 { assert!(p.enqueue_bounded(f, 5)); }
        assert_eq!(p.peak, 5);
        assert!(!p.enqueue_bounded(16, 5));
        assert!(!p.completed(14));
        assert!(!p.visible());
        assert!(p.completed(15));
        assert!(p.visible());
    }

    #[test]
    fn recording_lease_is_exclusive_and_reusable_only_by_owner() {
        let lease = RecordingLease::default();
        assert!(!lease.claim(0));
        assert!(lease.claim(11));
        assert!(!lease.claim(12));
        assert!(!lease.release(12));
        assert!(!lease.claim(13));
        assert!(lease.release(11));
        assert!(lease.claim(12));
        assert!(!lease.release(11));
        assert!(lease.release(12));
        lease.assert_idle();
    }

    #[test]
    fn rejects_reserved_fence_values() {
        let mut p = Progress::new();
        assert!(!p.sent(0));
        assert!(!p.sent(u64::MAX));
        assert!(p.sent(1));
        assert!(p.completed(1));
        assert!(p.visible());
    }
}

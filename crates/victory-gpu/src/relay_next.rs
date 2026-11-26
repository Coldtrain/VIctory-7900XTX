//! Relay Next, sprint R1: one synchronous, explicitly tracked submission.
//! Independent of the old relay flight queue. Preparation and title-memory
//! merging deliberately retain the working synchronous implementation.

use std::sync::OnceLock;
use windows::Win32::Foundation::{HANDLE, WAIT_OBJECT_0};
use windows::Win32::Graphics::Direct3D12::*;
use windows::Win32::System::Threading::{INFINITE, WaitForSingleObject};

pub(crate) fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| match std::env::var("VICTORY_RELAY_NEXT").as_deref() {
        Err(_) | Ok("0") => false,
        Ok("1") => true,
        Ok(mode) => panic!("VICTORY_RELAY_NEXT={mode} is not implemented; use 0 or 1"),
    })
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
}

impl Progress {
    fn new() -> Self {
        Self { phase: Phase::Preparing }
    }

    fn sent(&mut self, fence: u64) -> bool {
        if fence == 0 || fence == u64::MAX || !match self.phase {
            Phase::Preparing => true,
            Phase::GpuComplete(last) => fence > last,
            _ => false,
        } {
            return false;
        }
        self.phase = Phase::InFlight(fence);
        true
    }

    fn completed(&mut self, actual: u64) -> bool {
        let Phase::InFlight(target) = self.phase else { return false };
        if actual == u64::MAX || actual < target {
            return false;
        }
        self.phase = Phase::GpuComplete(target);
        true
    }

    fn visible(&mut self) -> bool {
        if !matches!(self.phase, Phase::GpuComplete(_)) {
            return false;
        }
        self.phase = Phase::CpuVisible;
        true
    }
}

/// A ticket spans the existing submission lock, including CPU copy-back.
/// It is not a flight queue: R1 never has two tickets executing concurrently.
pub(crate) struct Ticket {
    number: u64,
    progress: Progress,
    groups: usize,
    start: std::time::Instant,
}

impl Ticket {
    pub(crate) fn new(number: u64) -> Self {
        static START: std::sync::Once = std::sync::Once::new();
        START.call_once(|| tracing::info!(
            mode = 1, max_flights = 1, copy_queue = false,
            "Relay Next R1: synchronous scheduler active; legacy Relay disabled"
        ));
        Self { number, progress: Progress::new(), groups: 0, start: std::time::Instant::now() }
    }

    pub(crate) fn send(
        &mut self, device: &ID3D12Device, queue: &ID3D12CommandQueue,
        fence: &ID3D12Fence, event: HANDLE, target: u64,
        lists: &[Option<ID3D12CommandList>], queued: impl FnOnce(),
    ) {
        if !self.progress.sent(target) {
            crate::host::fail(format_args!("Relay Next invalid submission transition: {} {:?}", self.number, self.progress));
        }
        // SAFETY: the caller holds the submission lock; lists and their
        // backing allocations remain owned until synchronous completion.
        unsafe {
            device.GetDeviceRemovedReason().unwrap_or_else(|e| {
                crate::host::fail(format_args!("Relay Next device removed before submit {}: {e}", self.number))
            });
            queue.ExecuteCommandLists(lists);
            queued();
            queue.Signal(fence, target)
                .and_then(|()| fence.SetEventOnCompletion(target, event))
                .unwrap_or_else(|e| crate::host::fail(format_args!("Relay Next signal failed for {} fence {target}: {e}", self.number)));
        }
        self.groups += 1;
        if trace() {
            tracing::info!(submission = self.number, target, lists = lists.len(), "Relay Next submitted");
        }
    }

    pub(crate) fn wait(&mut self, device: &ID3D12Device, fence: &ID3D12Fence, event: HANDLE) {
        // SAFETY: the caller's event was registered by send under the lock.
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
                elapsed_us = self.start.elapsed().as_micros() as u64, "Relay Next CPU visible");
        }
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
    fn rejects_reserved_fence_values() {
        let mut p = Progress::new();
        assert!(!p.sent(0));
        assert!(!p.sent(u64::MAX));
        assert!(p.sent(1));
        assert!(p.completed(1));
        assert!(p.visible());
    }
}

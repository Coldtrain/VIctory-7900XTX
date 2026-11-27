//! Experimental, finite upper timer-step override for the retail build.
//! Do not change the shared literal or patch an unverified instruction.
//! Locate a unique decoded scalar min with a read-only 1/15-second operand
//! inside the documented timer update function, then redirect only its
//! displacement to a private read-only constant.

use anyhow::{Context, Result, ensure};
use iced_x86::{Decoder, DecoderOptions, Mnemonic};
use windows_sys::Win32::System::Diagnostics::Debug::{FlushInstructionCache, RtlLookupFunctionEntry};
use windows_sys::Win32::System::Memory::{
    MEM_COMMIT, MEM_IMAGE, MEM_RELEASE, MEMORY_BASIC_INFORMATION, PAGE_EXECUTE_READ,
    PAGE_EXECUTE_READWRITE, PAGE_READONLY, VirtualFree, VirtualProtect, VirtualQuery,
};
use windows_sys::Win32::System::Threading::GetCurrentProcess;

const TIMER_UPDATE: usize = 0x2226040;
const STOCK: u32 = (1.0f32 / 15.0).to_bits();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Candidate {
    ip: usize,
    next: usize,
    displacement: usize,
    operand: usize,
}

// Decode actual instruction boundaries; not a byte-pattern search.
// Reject invalid instructions and ambiguous matches.
fn candidates(code: &[u8], ip: usize, mut read: impl FnMut(usize) -> Option<u32>) -> Option<Vec<Candidate>> {
    let mut decoder = Decoder::with_ip(64, code, ip as u64, DecoderOptions::NONE);
    let mut found = Vec::new();
    while decoder.can_decode() {
        let ins = decoder.decode();
        if ins.is_invalid() {
            return None;
        }
        if !matches!(ins.mnemonic(), Mnemonic::Minss | Mnemonic::Vminss) || !ins.is_ip_rel_memory_operand() {
            continue;
        }
        let offsets = decoder.get_constant_offsets(&ins);
        let operand = ins.ip_rel_memory_address() as usize;
        if offsets.displacement_size() == 4 && read(operand) == Some(STOCK) {
            found.push(Candidate {
                ip: ins.ip() as usize,
                next: ins.next_ip() as usize,
                displacement: ins.ip() as usize + offsets.displacement_offset(),
                operand,
            });
        }
    }
    Some(found)
}

// Check the entire range is in one committed page region of this image.
// No reads of uncommitted, guard, or unrelated pages.
fn image_region(base: usize, address: usize, len: usize, code: bool) -> bool {
    let mut info: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: Windows writes the query result to a live local object.
    if unsafe { VirtualQuery(address as *const _, &mut info, std::mem::size_of::<MEMORY_BASIC_INFORMATION>()) } == 0 {
        return false;
    }
    let protection_ok = if code {
        matches!(info.Protect, PAGE_EXECUTE_READ | PAGE_EXECUTE_READWRITE)
    } else {
        info.Protect == PAGE_READONLY
    };
    info.State == MEM_COMMIT && info.Type == MEM_IMAGE && info.AllocationBase as usize == base
        && protection_ok && address.checked_add(len).is_some_and(|end|
            (info.BaseAddress as usize).checked_add(info.RegionSize).is_some_and(|limit| end <= limit))
}

/// Apply only when explicitly requested (100..=200 ms). Returns normally
/// without changing bytes if the compiled timer does not have the supported
/// direct-min instruction. The caller must check the original retail hash.
///
/// # Safety
/// The supplied title image remains mapped, before any title thread runs.
pub unsafe fn apply_max_step(base: usize, size: usize) -> Result<()> {
    let Some(spec) = std::env::var("VICTORY_SIM_MAX_MS").ok().filter(|v| !v.trim().is_empty() && v.trim() != "0") else {
        return Ok(());
    };
    let ms: f32 = spec.trim().parse().context("VICTORY_SIM_MAX_MS must be 100..=200")?;
    ensure!(ms.is_finite() && (100.0..=200.0).contains(&ms), "VICTORY_SIM_MAX_MS must be 100..=200");
    ensure!(TIMER_UPDATE < size, "timer update outside image");
    let at = base + TIMER_UPDATE;
    let mut image = 0;
    // SAFETY: lookup of a known RVA in the mapped title.
    let entry = unsafe { RtlLookupFunctionEntry(at as u64, &mut image, std::ptr::null_mut()) };
    if entry.is_null() || image != base as u64 {
        tracing::warn!("simulation max step unchanged: timer unwind entry unavailable");
        return Ok(());
    }
    let (begin, end) = unsafe { ((*entry).BeginAddress as usize, (*entry).EndAddress as usize) };
    if begin != TIMER_UPDATE || end <= begin || end > size || end - begin > 4096
        || !image_region(base, at, end - begin, true) {
        tracing::warn!(begin, end, "simulation max step unchanged: unexpected timer function bounds");
        return Ok(());
    }
    let code = unsafe { std::slice::from_raw_parts(at as *const u8, end - begin) };
    let read = |address: usize| {
        let rva = address.checked_sub(base)?;
        if rva.checked_add(4)? > size || !image_region(base, address, 4, false) {
            return None;
        }
        Some(unsafe { (address as *const u32).read_unaligned() })
    };
    let found = candidates(code, at, read).unwrap_or_default();
    if found.len() != 1 {
        let mut decoder = Decoder::with_ip(64, code, at as u64, DecoderOptions::NONE);
        // A compiler may load the limit into a register instead. Log all
        // direct references to its literal so a subsequent patch can be
        // verified without guessing a global constant.
        while decoder.can_decode() {
            let ins = decoder.decode();
            if ins.is_ip_rel_memory_operand() && read(ins.ip_rel_memory_address() as usize) == Some(STOCK) {
                tracing::info!(site = format_args!("{:#x}", ins.ip() as usize - base),
                    mnemonic = ?ins.mnemonic(), literal = format_args!("{:#x}", ins.ip_rel_memory_address() as usize - base),
                    "simulation max step: reference to stock upper-step literal");
            }
        }
        tracing::warn!(matches = found.len(), "simulation max step unchanged: no unique direct scalar minimum");
        return Ok(());
    }
    let c = found[0];
    let literal = crate::console_insn::near(c.ip).context("no nearby memory for simulation-step literal")?;
    let displacement = i32::try_from(literal as i64 - c.next as i64).context("simulation literal out of RIP range")?;
    // SAFETY: new private allocation, not shared with the game until patched.
    unsafe {
        (literal as *mut f32).write(ms / 1000.0);
        let mut old = 0;
        if VirtualProtect(literal as *const _, 4, PAGE_READONLY, &mut old) == 0 {
            VirtualFree(literal as *mut _, 0, MEM_RELEASE);
            anyhow::bail!("could not protect simulation-step literal");
        }
        if VirtualProtect(c.displacement as *const _, 4, PAGE_EXECUTE_READWRITE, &mut old) == 0 {
            VirtualFree(literal as *mut _, 0, MEM_RELEASE);
            anyhow::bail!("could not write simulation-step instruction");
        }
        (c.displacement as *mut i32).write_unaligned(displacement);
        let mut unused = 0;
        ensure!(VirtualProtect(c.displacement as *const _, 4, old, &mut unused) != 0,
            "could not restore timer instruction protection");
        ensure!(FlushInstructionCache(GetCurrentProcess(), c.ip as *const _, c.next - c.ip) != 0,
            "could not flush timer instruction");
    }
    // The private page remains alive for the mapped title's lifetime.
    tracing::info!(site = format_args!("{:#x}", c.ip - base), old_ms = 1000.0 / 15.0,
        new_ms = ms, "simulation max step patched (VICTORY_SIM_MAX_MS)");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_decoded_direct_min_of_exact_literal_matches() {
        // vminss xmm0,xmm0,[rip+0x19] (next IP 0x1008)
        let code = [0xc5, 0xfa, 0x5d, 0x05, 0x19, 0, 0, 0, 0xc3];
        let read = |at| (at == 0x1021).then_some(STOCK);
        let found = candidates(&code, 0x1000, read).unwrap();
        assert_eq!(found, [Candidate { ip: 0x1000, next: 0x1008, displacement: 0x1004, operand: 0x1021 }]);
        assert!(candidates(&code, 0x1000, |_| Some(0.1f32.to_bits())).unwrap().is_empty());
        // A VMOVSS of the same constant is not itself a clamp.
        let mut load = code;
        load[2] = 0x10;
        assert!(candidates(&load, 0x1000, read).unwrap().is_empty());
        let both = [code[..8].to_vec(), code.to_vec()].concat();
        assert_eq!(candidates(&both, 0x1000, |_| Some(STOCK)).unwrap().len(), 2);
    }
}

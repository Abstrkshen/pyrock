//! Stackful coroutines (generators and `async def` bodies), built on
//! `corosensei`: the body runs on its own stack and suspends with a value.

use corosensei::stack::DefaultStack;
use corosensei::{Coroutine, CoroutineResult, Yielder};
use std::mem::ManuallyDrop;

/// Each body gets this much (lazily committed) stack.
pub const STACK: usize = 2 << 20;

/// A coroutine that may be dropped while suspended (a generator left by
/// `break`, a task never resumed). With `panic = "unwind"` its frames are
/// unwound (destructors run). With `panic = "abort"` (release builds),
/// unwinding is impossible: the stack is reset and freed, and whatever its
/// frames held is leaked.
pub struct Co<I, Y, R> {
    co: ManuallyDrop<Coroutine<I, Y, R, DefaultStack>>,
}

impl<I: 'static, Y: 'static, R: 'static> Co<I, Y, R> {
    pub fn new(f: impl FnOnce(&Yielder<I, Y>, I) -> R + 'static) -> Result<Self, String> {
        let stack = match POOL.try_with(|p| p.borrow_mut().pop()).ok().flatten() {
            Some(s) => s,
            None => DefaultStack::new(STACK).map_err(|e| format!("coroutine stack: {e}"))?,
        };
        Ok(Co { co: ManuallyDrop::new(Coroutine::with_stack(stack, f)) })
    }

    pub fn resume(&mut self, input: I) -> CoroutineResult<Y, R> {
        self.co.resume(input)
    }
}

impl<I, Y, R> Drop for Co<I, Y, R> {
    fn drop(&mut self) {
        // SAFETY: taken exactly once, here; `self.co` is not used again.
        let mut co = unsafe { ManuallyDrop::take(&mut self.co) };
        if !co.done() {
            if !co.started() {
                // SAFETY: nothing has run, so nothing is on its stack.
                unsafe { co.force_reset() };
            } else {
                #[cfg(panic = "abort")]
                // SAFETY: frames on the coroutine stack are abandoned without
                // running their destructors (a leak, never a double free).
                unsafe {
                    co.force_reset()
                };
                #[cfg(not(panic = "abort"))]
                co.force_unwind();
            }
        }
        // Keep the stack for the next coroutine: a generator expression per
        // loop iteration would otherwise cost an mmap, an mprotect and a
        // munmap each time.
        let stack = co.into_stack();
        let _ = POOL.try_with(|p| {
            let mut p = p.borrow_mut();
            if p.len() < POOL_MAX {
                p.push(stack);
            }
        });
    }
}

/// How many free stacks a thread keeps.
const POOL_MAX: usize = 16;

thread_local! {
    static POOL: std::cell::RefCell<Vec<DefaultStack>> = const { std::cell::RefCell::new(Vec::new()) };
}

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
        let stack = DefaultStack::new(STACK).map_err(|e| format!("coroutine stack: {e}"))?;
        Ok(Co { co: ManuallyDrop::new(Coroutine::with_stack(stack, f)) })
    }

    pub fn resume(&mut self, input: I) -> CoroutineResult<Y, R> {
        self.co.resume(input)
    }
}

impl<I, Y, R> Drop for Co<I, Y, R> {
    fn drop(&mut self) {
        #[cfg(panic = "abort")]
        if self.co.started() && !self.co.done() {
            // SAFETY: frames on the coroutine stack are abandoned without
            // running their destructors (a leak, never a double free).
            unsafe { self.co.force_reset() };
        }
        // SAFETY: dropped exactly once, here.
        unsafe { ManuallyDrop::drop(&mut self.co) };
    }
}

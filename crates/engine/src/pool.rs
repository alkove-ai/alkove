// Native threads that share an instruction's independent parts; each part is computed as one thread alone would, so the bits never depend on the count.
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering::*};

static THREADS: AtomicUsize = AtomicUsize::new(1);

thread_local! {
    // the running session's own count, 0 when it has none and the process's applies
    static SESSION: Cell<usize> = const { Cell::new(0) };
}

// the count for sessions that set none
pub fn set_threads(n: usize) {
    THREADS.store(n.max(1), Relaxed);
}

pub fn threads() -> usize {
    match SESSION.with(Cell::get) {
        0 => THREADS.load(Relaxed),
        n => n,
    }
}

// a session's count while `f` runs on this thread
pub(crate) fn with<T>(n: usize, f: impl FnOnce() -> T) -> T {
    let before = SESSION.with(|c| c.replace(n));
    let out = f();
    SESSION.with(|c| c.set(before));
    out
}

pub fn max_threads() -> usize {
    if cfg!(target_arch = "wasm32") {
        return 1;
    }
    std::thread::available_parallelism().map_or(1, |n| n.get()).min(4)
}

#[cfg(target_arch = "wasm32")]
pub fn run(_: usize, parts: usize, f: &(dyn Fn(usize) + Sync)) {
    for i in 0..parts {
        f(i);
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use native::run;

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::any::Any;
    use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};
    use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering::*};
    use std::sync::{Condvar, Mutex, OnceLock};
    use std::time::{Duration, Instant};

    // an instruction follows another within microseconds, a hop 16 ms later
    const SPIN: Duration = Duration::from_micros(200);
    const BITS: u64 = 20;
    const MASK: u64 = (1 << BITS) - 1;

    type Job = &'static (dyn Fn(usize) + Sync);

    struct Shared {
        // generation, part count and next part in one word: a part is claimed only under the generation its closure was read for
        ticket: AtomicU64,
        job: [AtomicUsize; 2],
        done: AtomicUsize,
        // a part that panicked still counts as done; the caller panics with it once every part is over
        panic: Mutex<Option<Box<dyn Any + Send>>>,
        sleeping: Mutex<usize>,
        wake: Condvar,
        workers: Mutex<usize>,
        // one caller at a time; another runs its instruction alone
        caller: AtomicBool,
    }

    fn shared() -> &'static Shared {
        static S: OnceLock<Shared> = OnceLock::new();
        S.get_or_init(|| Shared {
            ticket: AtomicU64::new(0),
            job: [AtomicUsize::new(0), AtomicUsize::new(0)],
            done: AtomicUsize::new(0),
            panic: Mutex::new(None),
            sleeping: Mutex::new(0),
            wake: Condvar::new(),
            workers: Mutex::new(0),
            caller: AtomicBool::new(false),
        })
    }

    fn gen(t: u64) -> u64 {
        t >> (2 * BITS)
    }

    // parts of generation g until none is left; the closure was read after that generation was published and stays alive while a part of it is unfinished
    fn take(s: &Shared, g: u64, f: &(dyn Fn(usize) + Sync)) {
        let mut t = s.ticket.load(Acquire);
        loop {
            let (parts, i) = ((t >> BITS) & MASK, t & MASK);
            if gen(t) != g || i >= parts {
                return;
            }
            match s.ticket.compare_exchange_weak(t, t + 1, AcqRel, Acquire) {
                Ok(_) => {
                    if let Err(p) = catch_unwind(AssertUnwindSafe(|| f(i as usize))) {
                        s.panic.lock().unwrap().get_or_insert(p);
                    }
                    s.done.fetch_add(1, Release);
                    t = s.ticket.load(Acquire);
                }
                Err(now) => t = now,
            }
        }
    }

    fn worker(s: &'static Shared) {
        let mut seen = 0;
        loop {
            let t0 = Instant::now();
            while gen(s.ticket.load(Acquire)) == seen {
                if t0.elapsed() > SPIN {
                    let mut n = s.sleeping.lock().unwrap();
                    *n += 1;
                    while gen(s.ticket.load(Acquire)) == seen {
                        n = s.wake.wait(n).unwrap();
                    }
                    *n -= 1;
                    break;
                }
                std::hint::spin_loop();
            }
            seen = gen(s.ticket.load(Acquire));
            let words = [s.job[0].load(Relaxed), s.job[1].load(Relaxed)];
            // Safety: written before the ticket that published `seen`; a newer closure only replaces it once every part of `seen` is claimed and done, and then `take` claims none.
            let f: Job = unsafe { std::mem::transmute::<[usize; 2], Job>(words) };
            take(s, seen, f);
        }
    }

    // parts must write apart
    pub fn run(most: usize, parts: usize, f: &(dyn Fn(usize) + Sync)) {
        let threads = super::threads().min(most).min(parts);
        let s = shared();
        if threads <= 1 || parts as u64 > MASK || s.caller.swap(true, Acquire) {
            for i in 0..parts {
                f(i);
            }
            return;
        }
        {
            let mut w = s.workers.lock().unwrap();
            while *w + 1 < threads {
                std::thread::Builder::new().name("alkove-engine".into()).spawn(move || worker(s)).expect("an engine thread");
                *w += 1;
            }
        }
        // Safety: only the lifetime is erased; this call returns once every part is done.
        let words = unsafe { std::mem::transmute::<&(dyn Fn(usize) + Sync), [usize; 2]>(f) };
        s.job[0].store(words[0], Relaxed);
        s.job[1].store(words[1], Relaxed);
        s.done.store(0, Relaxed);
        let g = gen(s.ticket.load(Relaxed)) + 1;
        s.ticket.store((g << (2 * BITS)) | ((parts as u64) << BITS), Release);
        if *s.sleeping.lock().unwrap() > 0 {
            s.wake.notify_all();
        }
        take(s, g, f);
        while s.done.load(Acquire) < parts {
            std::hint::spin_loop();
        }
        let panic = s.panic.lock().unwrap().take();
        s.caller.store(false, Release);
        if let Some(p) = panic {
            resume_unwind(p);
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[test]
    fn a_part_that_panics_reaches_the_caller() {
        let r = std::panic::catch_unwind(|| with(4, || run(4, 16, &|i| assert!(i != 9, "part 9"))));
        assert!(r.is_err());
        let sum = AtomicUsize::new(0);
        with(4, || run(4, 16, &|i| _ = sum.fetch_add(i, Relaxed)));
        assert_eq!(sum.load(Relaxed), 120);
    }
}

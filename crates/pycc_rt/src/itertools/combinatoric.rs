//! `product`, `permutations`, `combinations` and
//! `combinations_with_replacement`: each takes its pools when it is called
//! (CPython's `tuple(iterable)`), then yields one tuple of pool elements
//! per step, in CPython's order, computing it only when asked for.

use super::{Input, Item, Native, Pool, flag, spawn, spawn_empty};
use crate::generator::PyGenObj;

/// The tuple of pool elements `indices` picks.
fn pick(pools: &[&Pool], indices: &[usize]) -> Item {
    Item::Fields(
        pools
            .iter()
            .zip(indices)
            .map(|(pool, &i)| pool.get(i))
            .collect(),
    )
}

// ---------------------------------------------------------------- product

struct Product {
    pools: Vec<Pool>,
    /// Which pool each position draws from (`repeat` repeats the list).
    order: Vec<usize>,
    indices: Vec<usize>,
    started: bool,
    done: bool,
}

impl Native for Product {
    fn next(&mut self) -> Option<Item> {
        if self.done {
            return None;
        }
        if self.started {
            // Advance the rightmost index that can, resetting those after it.
            let mut position = self.indices.len();
            loop {
                if position == 0 {
                    self.done = true;
                    return None;
                }
                position -= 1;
                self.indices[position] += 1;
                if self.indices[position] < self.pools[self.order[position]].len() {
                    break;
                }
                self.indices[position] = 0;
            }
        } else {
            self.started = true;
            if self.order.iter().any(|&p| self.pools[p].len() == 0) {
                self.done = true;
                return None;
            }
        }
        let pools: Vec<&Pool> = self.order.iter().map(|&p| &self.pools[p]).collect();
        Some(pick(&pools, &self.indices))
    }
}

const PRODUCT: &[u8] = b"itertools.product\0";

fn product(inputs: Vec<Input>, repeat: i64, out: i64) -> *mut PyGenObj {
    let mut pools = Vec::with_capacity(inputs.len());
    for input in inputs {
        match Pool::take(input) {
            Some(pool) => pools.push(pool),
            None => return spawn_empty(out, PRODUCT),
        }
    }
    let repeat = usize::try_from(flag(repeat)).unwrap_or(0);
    let order: Vec<usize> = (0..repeat).flat_map(|_| 0..pools.len()).collect();
    let indices = vec![0; order.len()];
    spawn(
        Box::new(Product {
            pools,
            order,
            indices,
            started: false,
            done: false,
        }),
        out,
        PRODUCT,
    )
}

/// `itertools.product(a, repeat=repeat)`.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_it_product1(a: *mut u8, repeat: i64, da: i64, out: i64) -> *mut PyGenObj {
    product(vec![Input::new(a, da)], repeat, out)
}

/// `itertools.product(a, b, repeat=repeat)`.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_it_product2(
    a: *mut u8,
    b: *mut u8,
    repeat: i64,
    da: i64,
    db: i64,
    out: i64,
) -> *mut PyGenObj {
    product(vec![Input::new(a, da), Input::new(b, db)], repeat, out)
}

/// `itertools.product(a, b, c, repeat=repeat)`.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_it_product3(
    a: *mut u8,
    b: *mut u8,
    c: *mut u8,
    repeat: i64,
    da: i64,
    db: i64,
    dc: i64,
    out: i64,
) -> *mut PyGenObj {
    product(
        vec![Input::new(a, da), Input::new(b, db), Input::new(c, dc)],
        repeat,
        out,
    )
}

/// `itertools.product(a, b, c, d, repeat=repeat)`.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "C" fn pycc_rt_it_product4(
    a: *mut u8,
    b: *mut u8,
    c: *mut u8,
    d: *mut u8,
    repeat: i64,
    da: i64,
    db: i64,
    dc: i64,
    dd: i64,
    out: i64,
) -> *mut PyGenObj {
    product(
        vec![
            Input::new(a, da),
            Input::new(b, db),
            Input::new(c, dc),
            Input::new(d, dd),
        ],
        repeat,
        out,
    )
}

// ----------------------------------------------------------- permutations

struct Permutations {
    pool: Pool,
    r: usize,
    indices: Vec<usize>,
    cycles: Vec<usize>,
    started: bool,
    done: bool,
}

impl Permutations {
    fn current(&self) -> Item {
        let pools = vec![&self.pool; self.r];
        pick(&pools, &self.indices[..self.r])
    }
}

impl Native for Permutations {
    fn next(&mut self) -> Option<Item> {
        if self.done {
            return None;
        }
        let n = self.pool.len();
        if !self.started {
            self.started = true;
            if self.r > n {
                self.done = true;
                return None;
            }
            return Some(self.current());
        }
        for i in (0..self.r).rev() {
            self.cycles[i] -= 1;
            if self.cycles[i] == 0 {
                let moved = self.indices.remove(i);
                self.indices.push(moved);
                self.cycles[i] = n - i;
            } else {
                let j = self.cycles[i];
                self.indices.swap(i, n - j);
                return Some(self.current());
            }
        }
        self.done = true;
        None
    }
}

const PERMUTATIONS: &[u8] = b"itertools.permutations\0";

/// `itertools.permutations(it, r)` (`r >= 0`, checked by the compiler).
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_it_permutations(
    it: *mut u8,
    r: i64,
    source: i64,
    out: i64,
) -> *mut PyGenObj {
    let Some(pool) = Pool::take(Input::new(it, source)) else {
        return spawn_empty(out, PERMUTATIONS);
    };
    let n = pool.len();
    let r = usize::try_from(flag(r)).unwrap_or(0);
    let cycles = if r <= n {
        (n - r + 1..=n).rev().collect()
    } else {
        Vec::new()
    };
    spawn(
        Box::new(Permutations {
            pool,
            r,
            indices: (0..n).collect(),
            cycles,
            started: false,
            done: false,
        }),
        out,
        PERMUTATIONS,
    )
}

// ----------------------------------------------------------- combinations

struct Combinations {
    pool: Pool,
    r: usize,
    indices: Vec<usize>,
    replacement: bool,
    started: bool,
    done: bool,
}

impl Combinations {
    fn current(&self) -> Item {
        let pools = vec![&self.pool; self.r];
        pick(&pools, &self.indices)
    }
}

impl Native for Combinations {
    fn next(&mut self) -> Option<Item> {
        if self.done {
            return None;
        }
        let (n, r) = (self.pool.len(), self.r);
        if !self.started {
            self.started = true;
            let empty = if self.replacement {
                n == 0 && r > 0
            } else {
                r > n
            };
            if empty {
                self.done = true;
                return None;
            }
            return Some(self.current());
        }
        let found = (0..r).rev().find(|&i| {
            if self.replacement {
                self.indices[i] != n - 1
            } else {
                self.indices[i] != i + n - r
            }
        });
        let Some(i) = found else {
            self.done = true;
            return None;
        };
        if self.replacement {
            let value = self.indices[i] + 1;
            for index in &mut self.indices[i..] {
                *index = value;
            }
        } else {
            self.indices[i] += 1;
            for j in i + 1..r {
                self.indices[j] = self.indices[j - 1] + 1;
            }
        }
        Some(self.current())
    }
}

fn combinations(it: *mut u8, r: i64, source: i64, out: i64, replacement: bool) -> *mut PyGenObj {
    let name: &'static [u8] = if replacement {
        b"itertools.combinations_with_replacement\0"
    } else {
        b"itertools.combinations\0"
    };
    let Some(pool) = Pool::take(Input::new(it, source)) else {
        return spawn_empty(out, name);
    };
    let r = usize::try_from(flag(r)).unwrap_or(0);
    let indices = if replacement {
        vec![0; r]
    } else {
        (0..r).collect()
    };
    spawn(
        Box::new(Combinations {
            pool,
            r,
            indices,
            replacement,
            started: false,
            done: false,
        }),
        out,
        name,
    )
}

/// `itertools.combinations(it, r)`.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_it_combinations(
    it: *mut u8,
    r: i64,
    source: i64,
    out: i64,
) -> *mut PyGenObj {
    combinations(it, r, source, out, false)
}

/// `itertools.combinations_with_replacement(it, r)`.
#[unsafe(no_mangle)]
pub extern "C" fn pycc_rt_it_combinations_with_replacement(
    it: *mut u8,
    r: i64,
    source: i64,
    out: i64,
) -> *mut PyGenObj {
    combinations(it, r, source, out, true)
}

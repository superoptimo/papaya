// Adapted from: https://github.com/jonhoo/flurry/tree/main/tests/jdk

use papaya::HashMap;
use rand::prelude::*;

use std::sync::Barrier;
use std::thread;
use std::{hash::Hash, ops::Range};

mod common;
use common::{threads, with_map};

// Call `contains_key` in parallel for a shared set of keys.
#[test]
fn contains_key_stress() {
    const ENTRIES: usize = match () {
        _ if cfg!(miri) => 64,
        _ if resize_stress!() => 1 << 12,
        _ => 1 << 14,
    };
    const ITERATIONS: usize = if cfg!(miri) { 1 } else { 64 };

    with_map(|map| {
        for _ in (0..ITERATIONS).inspect(|e| debug!("{e}/{ITERATIONS}")) {
            let map = map();
            let mut content = [0; ENTRIES];

            {
                let guard = map.guard();
                for k in 0..ENTRIES {
                    map.insert(k, k, &guard);
                    content[k] = k;
                }
            }

            let threads = threads();
            let barrier = Barrier::new(threads);
            thread::scope(|s| {
                for _ in 0..threads {
                    s.spawn(|| {
                        barrier.wait();
                        let guard = map.guard();
                        for i in 0..ENTRIES {
                            let key = content[i % content.len()];
                            assert!(map.contains_key(&key, &guard));
                        }
                    });
                }
            });
        }
    });
}

// Call `insert` in parallel with each thread inserting a distinct set of keys.
#[test]
fn insert_stress() {
    const ENTRIES: usize = match () {
        _ if cfg!(miri) => 64,
        _ if resize_stress!() => 1 << 11,
        _ => 1 << 14,
    };
    const ITERATIONS: usize = if cfg!(miri) { 1 } else { 64 };

    #[derive(Hash, PartialEq, Eq, Clone, Copy)]
    struct KeyVal {
        _data: usize,
    }

    impl KeyVal {
        pub fn new() -> Self {
            let mut rng = rand::thread_rng();
            Self { _data: rng.gen() }
        }
    }

    with_map(|map| {
        for _ in (0..ITERATIONS).inspect(|e| debug!("{e}/{ITERATIONS}")) {
            let map = map();
            let threads = threads();
            let barrier = Barrier::new(threads);
            thread::scope(|s| {
                for _ in 0..threads {
                    s.spawn(|| {
                        barrier.wait();
                        for _ in 0..ENTRIES {
                            let key = KeyVal::new();
                            map.insert(key, key, &map.guard());
                            assert!(map.contains_key(&key, &map.guard()));
                        }
                    });
                }
            });
            assert_eq!(map.len(), ENTRIES * threads);
        }
    });
}

// Call `update` in parallel for a shared set of keys.
#[test]
fn update_stress() {
    const ENTRIES: usize = match () {
        _ if cfg!(miri) => 64,
        _ if resize_stress!() => 1 << 12,
        _ => 1 << 14,
    };
    const ITERATIONS: usize = if cfg!(miri) { 1 } else { 64 };

    with_map(|map| {
        for _ in (0..ITERATIONS).inspect(|e| debug!("{e}/{ITERATIONS}")) {
            let map = map();

            {
                let guard = map.guard();
                for i in 0..ENTRIES {
                    map.insert(i, 0, &guard);
                }
            }

            let threads = threads();
            let barrier = Barrier::new(threads);

            thread::scope(|s| {
                for _ in 0..threads {
                    s.spawn(|| {
                        barrier.wait();
                        let guard = map.guard();
                        for i in 0..ENTRIES {
                            let new = *map.update(i, |v| v + 1, &guard).unwrap();
                            assert!((0..=threads).contains(&new));
                        }
                    });
                }
            });

            let guard = map.guard();
            for i in 0..ENTRIES {
                assert_eq!(*map.get(&i, &guard).unwrap(), threads);
            }
        }
    });
}

// Call `update` in parallel for a shared set of keys, with a single thread dedicated
// to calling `insert`. This is likely to cause interference with incremental resizing.
#[test]
fn update_insert_stress() {
    const ENTRIES: usize = match () {
        _ if cfg!(miri) => 64,
        _ if resize_stress!() => 1 << 12,
        _ => 1 << 14,
    };
    const ITERATIONS: usize = if cfg!(miri) { 1 } else { 64 };

    with_map(|map| {
        let map = map();

        {
            let guard = map.guard();
            for i in 0..ENTRIES {
                map.insert(i, 0, &guard);
            }
        }

        for t in (0..ITERATIONS).inspect(|e| debug!("{e}/{ITERATIONS}")) {
            let threads = threads();
            let barrier = Barrier::new(threads);

            let threads = &threads;
            thread::scope(|s| {
                for _ in 0..(threads - 1) {
                    s.spawn(|| {
                        barrier.wait();
                        let guard = map.guard();
                        for i in 0..ENTRIES {
                            let new = *map.update(i, |v| v + 1, &guard).unwrap();
                            assert!((0..=(threads * (t + 1))).contains(&new));
                        }
                    });
                }

                s.spawn(|| {
                    barrier.wait();
                    let guard = map.guard();
                    for i in ENTRIES..(ENTRIES * 3) {
                        map.insert(i, usize::MAX, &guard);
                    }
                });
            });

            let guard = map.guard();
            for i in 0..ENTRIES {
                assert_eq!(*map.get(&i, &guard).unwrap(), (threads - 1) * (t + 1));
            }

            for i in ENTRIES..(ENTRIES * 3) {
                assert_eq!(*map.get(&i, &guard).unwrap(), usize::MAX);
            }
        }
    });
}

// Performs a mix of operations with each thread operating on a distinct set of keys.
#[test]
fn mixed_chunk_stress() {
    const ENTRIES: usize = match () {
        _ if cfg!(miri) => 48,
        _ if resize_stress!() => 1 << 10,
        _ => 1 << 14,
    };
    const ITERATIONS: usize = if cfg!(miri) { 1 } else { 48 };

    let run =
        |barrier: &Barrier, chunk: Range<usize>, map: &HashMap<usize, usize>, threads: usize| {
            barrier.wait();

            for i in chunk.clone() {
                assert_eq!(map.pin().insert(i, i + 1), None);
            }

            for i in chunk.clone() {
                assert_eq!(map.pin().get(&i), Some(&(i + 1)));
            }

            for i in chunk.clone() {
                assert_eq!(map.pin().update(i, |i| i - 1), Some(&i));
            }

            for i in chunk.clone() {
                assert_eq!(map.pin().remove(&i), Some(&i));
            }

            for i in chunk.clone() {
                assert_eq!(map.pin().get(&i), None);
            }

            for i in chunk.clone() {
                assert_eq!(map.pin().insert(i, i + 1), None);
            }

            for i in chunk.clone() {
                assert_eq!(map.pin().get(&i), Some(&(i + 1)));
            }

            if !resize_stress!() {
                for (&k, &v) in map.pin().iter() {
                    assert!(k < ENTRIES * threads);
                    assert!(v == k || v == k + 1);
                }
            }
        };

    with_map(|map| {
        for _ in (0..ITERATIONS).inspect(|e| debug!("{e}/{ITERATIONS}")) {
            let map = map();
            let threads = threads();
            let barrier = Barrier::new(threads);

            thread::scope(|s| {
                for i in 0..threads {
                    let map = &map;
                    let barrier = &barrier;

                    let chunk = (ENTRIES * i)..(ENTRIES * (i + 1));
                    s.spawn(move || run(barrier, chunk, map, threads));
                }
            });

            if !resize_stress!() {
                let v: Vec<_> = (0..ENTRIES * threads).map(|i| (i, i + 1)).collect();
                let mut got: Vec<_> = map.pin().iter().map(|(&k, &v)| (k, v)).collect();
                got.sort();
                assert_eq!(v, got);
            }

            assert_eq!(map.len(), ENTRIES * threads);
        }
    });
}

// Performs a mix of operations with each thread operating on a specific entry within
// a distinct set of keys. This is more likely to cause interference with incremental resizing.
#[test]
fn mixed_entry_stress() {
    const ENTRIES: usize = match () {
        _ if cfg!(miri) => 100,
        _ if resize_stress!() => 1 << 10,
        _ => 1 << 10,
    };
    const OPERATIONS: usize = if cfg!(miri) { 1 } else { 72 };
    const ITERATIONS: usize = if cfg!(miri) { 1 } else { 48 };

    let run =
        |barrier: &Barrier, chunk: Range<usize>, map: &HashMap<usize, usize>, threads: usize| {
            barrier.wait();

            for i in chunk.clone() {
                for _ in 0..OPERATIONS {
                    assert_eq!(map.pin().insert(i, i + 1), None);
                    assert_eq!(map.pin().get(&i), Some(&(i + 1)));
                    assert_eq!(map.pin().update(i, |i| i + 1), Some(&(i + 2)));
                    assert_eq!(map.pin().remove(&i), Some(&(i + 2)));
                    assert_eq!(map.pin().get(&i), None);
                    assert_eq!(map.pin().update(i, |i| i + 1), None);
                }
            }

            for i in chunk.clone() {
                assert_eq!(map.pin().get(&i), None);
            }

            if !resize_stress!() {
                for (&k, &v) in map.pin().iter() {
                    assert!(k < ENTRIES * threads);
                    assert!(v == k + 1 || v == k + 2);
                }
            }
        };

    with_map(|map| {
        for _ in (0..ITERATIONS).inspect(|e| debug!("{e}/{ITERATIONS}")) {
            let map = map();
            let threads = threads();
            let barrier = Barrier::new(threads);

            thread::scope(|s| {
                for i in 0..threads {
                    let map = &map;
                    let barrier = &barrier;

                    let chunk = (ENTRIES * i)..(ENTRIES * (i + 1));
                    s.spawn(move || run(barrier, chunk, map, threads));
                }
            });

            if !resize_stress!() {
                let got: Vec<_> = map.pin().iter().map(|(&k, &v)| (k, v)).collect();
                assert_eq!(got, []);
            }
            assert_eq!(map.len(), 0);
        }
    });
}

// Performs a mix of operations on a single thread.
#[test]
fn everything() {
    const SIZE: usize = match () {
        _ if cfg!(miri) => 1 << 5,
        _ if resize_stress!() => 1 << 8,
        _ => 1 << 16,
    };
    // there must be more things absent than present!
    const ABSENT_SIZE: usize = if cfg!(miri) { 1 << 5 } else { 1 << 17 };
    const ABSENT_MASK: usize = ABSENT_SIZE - 1;

    let mut rng = rand::thread_rng();

    with_map(|map| {
        let map = map();
        let mut keys: Vec<_> = (0..ABSENT_SIZE + SIZE).collect();
        keys.shuffle(&mut rng);
        let absent_keys = &keys[0..ABSENT_SIZE];
        let keys = &keys[ABSENT_SIZE..];

        // put (absent)
        t3(&map, keys, SIZE);
        // put (present)
        t3(&map, keys, 0);
        // contains_key (present & absent)
        t7(&map, keys, absent_keys);
        // contains_key (present)
        t4(&map, keys, SIZE);
        // contains_key (absent)
        t4(&map, absent_keys, 0);
        // get
        t6(&map, keys, absent_keys, SIZE, ABSENT_MASK);
        // get (present)
        t1(&map, keys, SIZE);
        // get (absent)
        t1(&map, absent_keys, 0);
        // remove (absent)
        t2(&map, absent_keys, 0);
        // remove (present)
        t5(&map, keys, SIZE / 2);
        // put (half present)
        t3(&map, keys, SIZE / 2);

        // iter, keys, values (present)
        if !resize_stress!() {
            ittest1(&map, SIZE);
            ittest2(&map, SIZE);
            ittest3(&map, SIZE);
        }
    });

    fn t1<K, V>(map: &HashMap<K, V>, keys: &[K], expect: usize)
    where
        K: Sync + Send + Clone + Hash + Ord,
        V: Sync + Send,
    {
        let mut sum = 0;
        let iters = 4;
        let guard = map.guard();
        for _ in 0..iters {
            for key in keys {
                if map.get(key, &guard).is_some() {
                    sum += 1;
                }
            }
        }
        assert_eq!(sum, expect * iters);
    }

    fn t2<K>(map: &HashMap<K, usize>, keys: &[K], expect: usize)
    where
        K: Sync + Send + Copy + Hash + Ord + std::fmt::Display,
    {
        let mut sum = 0;
        let guard = map.guard();
        for key in keys {
            if map.remove(key, &guard).is_some() {
                sum += 1;
            }
        }
        assert_eq!(sum, expect);
    }

    fn t3<K>(map: &HashMap<K, usize>, keys: &[K], expect: usize)
    where
        K: Sync + Send + Copy + Hash + Ord,
    {
        let mut sum = 0;
        let guard = map.guard();
        for i in 0..keys.len() {
            if map.insert(keys[i], 0, &guard).is_none() {
                sum += 1;
            }
        }
        assert_eq!(sum, expect);
    }

    fn t4<K>(map: &HashMap<K, usize>, keys: &[K], expect: usize)
    where
        K: Sync + Send + Copy + Hash + Ord,
    {
        let mut sum = 0;
        let guard = map.guard();
        for i in 0..keys.len() {
            if map.contains_key(&keys[i], &guard) {
                sum += 1;
            }
        }
        assert_eq!(sum, expect);
    }

    fn t5<K>(map: &HashMap<K, usize>, keys: &[K], expect: usize)
    where
        K: Sync + Send + Copy + Hash + Ord,
    {
        let mut sum = 0;
        let guard = map.guard();
        let mut i = keys.len() as isize - 2;
        while i >= 0 {
            if map.remove(&keys[i as usize], &guard).is_some() {
                sum += 1;
            }
            i -= 2;
        }
        assert_eq!(sum, expect);
    }

    fn t6<K, V>(map: &HashMap<K, V>, keys1: &[K], keys2: &[K], expect: usize, mask: usize)
    where
        K: Sync + Send + Clone + Hash + Ord,
        V: Sync + Send,
    {
        let mut sum = 0;
        let guard = map.guard();
        for i in 0..expect {
            if map.get(&keys1[i], &guard).is_some() {
                sum += 1;
            }
            if map.get(&keys2[i & mask], &guard).is_some() {
                sum += 1;
            }
        }
        assert_eq!(sum, expect);
    }

    fn t7<K>(map: &HashMap<K, usize>, k1: &[K], k2: &[K])
    where
        K: Sync + Send + Copy + Hash + Ord,
    {
        let mut sum = 0;
        let guard = map.guard();
        for i in 0..k1.len() {
            if map.contains_key(&k1[i], &guard) {
                sum += 1;
            }
            if map.contains_key(&k2[i], &guard) {
                sum += 1;
            }
        }
        assert_eq!(sum, k1.len());
    }

    fn ittest1<K>(map: &HashMap<K, usize>, expect: usize)
    where
        K: Sync + Send + Copy + Hash + Eq,
    {
        let mut sum = 0;
        let guard = map.guard();
        for _ in map.keys(&guard) {
            sum += 1;
        }
        assert_eq!(sum, expect);
    }

    fn ittest2<K>(map: &HashMap<K, usize>, expect: usize)
    where
        K: Sync + Send + Copy + Hash + Eq,
    {
        let mut sum = 0;
        let guard = map.guard();
        for _ in map.values(&guard) {
            sum += 1;
        }
        assert_eq!(sum, expect);
    }

    fn ittest3<K>(map: &HashMap<K, usize>, expect: usize)
    where
        K: Sync + Send + Copy + Hash + Eq,
    {
        let mut sum = 0;
        let guard = map.guard();
        for _ in map.iter(&guard) {
            sum += 1;
        }
        assert_eq!(sum, expect);
    }
}

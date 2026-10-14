//! Native `itertools` iterators and `collections` containers: every value
//! they hold or hand out is released.

use super::assert_constant_memory;

probe!(
    itertools_permutations_of_ints,
    "for p in itertools.permutations([i, 1, 2], 2):\n    total += p[0]"
);
probe!(
    itertools_product_of_strs,
    "for a, b in itertools.product([mk(i), \"x\"], \"ab\"):\n    total += len(a + b)"
);
probe!(
    itertools_early_exit_releases_the_pool,
    "for p in itertools.permutations([mk(i), \"a\", \"b\"]):\n    total += len(p[0])\n    break"
);
probe!(
    itertools_groupby_groups,
    "for k, g in itertools.groupby([mk(i), mk(i), \"z\"]):\n    total += len(list(g)) + len(k)"
);
probe!(
    itertools_islice_of_count,
    "for v in itertools.islice(itertools.count(i), 1, 4):\n    total += v"
);
probe!(
    itertools_chain_of_a_generator,
    "for v in itertools.chain(gstr(i), [mk(i)]):\n    total += len(v)"
);
probe!(
    itertools_accumulate_strs,
    "for v in itertools.accumulate([mk(i), \"a\"], initial=\"w\"):\n    total += len(v)"
);
probe!(
    itertools_pairwise_of_instances,
    "for a, b in itertools.pairwise([mkp(i), mkp(1), mkp(2)]):\n    total += a.x + b.x"
);
probe!(
    itertools_comprehension_over_combinations,
    "total += len([c for c in itertools.combinations([mk(i), \"a\", \"b\"], 2)])"
);
probe!(
    itertools_chain_from_iterable_of_lists,
    "total += sum(itertools.chain.from_iterable([[i], [1, 2]]))"
);

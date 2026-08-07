//! What compiler chiero says it is, when a header asks.
//!
//! `__has_attribute` and `__has_builtin` are not questions about chiero. chiero's predefine set
//! is an **impersonation of the build compiler** — `__GNUC__` is baked, and `chiero-cli`'s
//! frontend captures the whole `cc -dM` at run time so headers configure for the code that
//! actually ships. Answering these from chiero's own capabilities (it acts on four attributes
//! and models a handful of builtins) would make every system header configure for a compiler
//! that has never existed, and chiero would then analyse a program nobody compiles.
//!
//! So the answers are **gcc 13's**, and [`TABLE`] records them name by name.
//!
//! # Why a table and not a rule
//!
//! There is no rule. `packed` is supported and `minsize` is not; `__builtin_bswap128` is and
//! `__builtin_bit_cast` is not; the `__x__` spelling is accepted for the attributes gcc has and
//! not conjured for the ones it lacks. Forty-six of the entries here are **`false`**, and they are
//! the load-bearing ones: they are what makes this a table rather than a rubber stamp, and
//! `__init_priority__` in particular is queried by a real header on this machine.
//!
//! # The two things that keep it honest
//!
//! A name **in** the table answered `false` is knowledge — gcc agrees, and nothing is reported.
//! A name **absent** is ignorance: the query answers `0`, because `#if` must yield a number and
//! there is no third value, and the guess is recorded as a diagnostic naming the name so a
//! reader can extend this list. That is the in-band/out-of-band split the whole project runs on
//! (`Selection::NeedsAst`, `Tier1Report::unreadable`): the number cannot carry "I do not know",
//! so something beside it must.
//!
//! And `crates/chiero-pp/tests/feature_queries.rs` re-asks **gcc** for every row. A table of
//! remembered answers nobody re-checks is a table that drifts silently into deciding which
//! branch every system header takes.

/// `(query, name, gcc 13's answer)`. Measured, not recalled — see the module docs.
pub const TABLE: &[(&str, &str, bool)] = &[
    ("__has_attribute", "access", true),
    ("__has_attribute", "__aligned__", true),
    ("__has_attribute", "aligned", true),
    ("__has_attribute", "__alloc_align__", true),
    ("__has_attribute", "alloc_align", true),
    ("__has_attribute", "alloc_size", true),
    ("__has_attribute", "__always_inline__", true),
    ("__has_attribute", "always_inline", true),
    ("__has_attribute", "__artificial__", true),
    ("__has_attribute", "artificial", true),
    ("__has_attribute", "__assume__", true),
    ("__has_attribute", "__attribute_deprecated_with_message__", false),
    ("__has_attribute", "__bases", false),
    ("__has_attribute", "c_generic_selections", false),
    ("__has_attribute", "__cleanup__", true),
    ("__has_attribute", "cleanup", true),
    ("__has_attribute", "__cold__", true),
    ("__has_attribute", "cold", true),
    ("__has_attribute", "__const__", true),
    ("__has_attribute", "const", true),
    ("__has_attribute", "__constructor__", true),
    ("__has_attribute", "constructor", true),
    ("__has_attribute", "__copy__", true),
    ("__has_attribute", "copy", true),
    ("__has_attribute", "__deprecated__", true),
    ("__has_attribute", "deprecated", true),
    ("__has_attribute", "__destructor__", true),
    ("__has_attribute", "destructor", true),
    ("__has_attribute", "__direct_bases", false),
    ("__has_attribute", "enable_if", false),
    ("__has_attribute", "__fallthrough__", true),
    ("__has_attribute", "fallthrough", true),
    ("__has_attribute", "__format__", true),
    ("__has_attribute", "format", true),
    ("__has_attribute", "__format_arg__", true),
    ("__has_attribute", "__has_unique_object_representations", false),
    ("__has_attribute", "hot", true),
    ("__has_attribute", "__indirect_return__", true),
    ("__has_attribute", "__init_priority__", false),
    ("__has_attribute", "__is_convertible", false),
    ("__has_attribute", "__is_layout_compatible", false),
    ("__has_attribute", "__is_nothrow_convertible", false),
    ("__has_attribute", "__is_pointer_interconvertible_base_of", false),
    ("__has_attribute", "leaf", true),
    ("__has_attribute", "__make_integer_seq", false),
    ("__has_attribute", "__malloc__", true),
    ("__has_attribute", "malloc", true),
    ("__has_attribute", "__may_alias__", true),
    ("__has_attribute", "may_alias", true),
    ("__has_attribute", "minsize", false),
    ("__has_attribute", "__mode__", true),
    ("__has_attribute", "mode", true),
    ("__has_attribute", "noclone", true),
    ("__has_attribute", "nodebug", false),
    ("__has_attribute", "__noinline__", true),
    ("__has_attribute", "noinline", true),
    ("__has_attribute", "__nonnull__", true),
    ("__has_attribute", "nonnull", true),
    ("__has_attribute", "noplt", true),
    ("__has_attribute", "no_profile_instrument_function", true),
    ("__has_attribute", "__noreturn__", true),
    ("__has_attribute", "noreturn", true),
    ("__has_attribute", "no_sanitize", true),
    ("__has_attribute", "__nothrow__", true),
    ("__has_attribute", "nothrow", true),
    ("__has_attribute", "__packed__", true),
    ("__has_attribute", "packed", true),
    ("__has_attribute", "preferred_type", false),
    ("__has_attribute", "__pure__", true),
    ("__has_attribute", "pure", true),
    ("__has_attribute", "__reference_constructs_from_temporary", false),
    ("__has_attribute", "__reference_converts_from_temporary", false),
    ("__has_attribute", "__remove_cv", false),
    ("__has_attribute", "__remove_cvref", false),
    ("__has_attribute", "__remove_reference", false),
    ("__has_attribute", "__returns_nonnull__", true),
    ("__has_attribute", "returns_nonnull", true),
    ("__has_attribute", "returns_twice", true),
    ("__has_attribute", "__section__", true),
    ("__has_attribute", "section", true),
    ("__has_attribute", "sentinel", true),
    ("__has_attribute", "symver", true),
    ("__has_attribute", "__transparent_union__", true),
    ("__has_attribute", "transparent_union", true),
    ("__has_attribute", "__unused__", true),
    ("__has_attribute", "unused", true),
    ("__has_attribute", "__used__", true),
    ("__has_attribute", "used", true),
    ("__has_attribute", "__vector_size__", true),
    ("__has_attribute", "vector_size", true),
    ("__has_attribute", "__visibility__", true),
    ("__has_attribute", "visibility", true),
    ("__has_attribute", "__warn_unused_result__", true),
    ("__has_attribute", "warn_unused_result", true),
    ("__has_attribute", "__weak__", true),
    ("__has_attribute", "weak", true),
    ("__has_builtin", "__builtin_add_overflow", true),
    ("__has_builtin", "__builtin_alloca", true),
    ("__has_builtin", "__builtin_assume_aligned", true),
    ("__has_builtin", "__builtin_bit_cast", false),
    ("__has_builtin", "__builtin_bitreverse16", false),
    ("__has_builtin", "__builtin_bitreverse32", false),
    ("__has_builtin", "__builtin_bitreverse64", false),
    ("__has_builtin", "__builtin_bitreverse8", false),
    ("__has_builtin", "__builtin_bswap128", true),
    ("__has_builtin", "__builtin_bswap16", true),
    ("__has_builtin", "__builtin_bswap32", true),
    ("__has_builtin", "__builtin_bswap64", true),
    ("__has_builtin", "__builtin_choose_expr", true),
    ("__has_builtin", "__builtin_clear_padding", true),
    ("__has_builtin", "__builtin_clz", true),
    ("__has_builtin", "__builtin_clzl", true),
    ("__has_builtin", "__builtin_clzll", true),
    ("__has_builtin", "__builtin_constant_p", true),
    ("__has_builtin", "__builtin_ctz", true),
    ("__has_builtin", "__builtin_ctzl", true),
    ("__has_builtin", "__builtin_ctzll", true),
    ("__has_builtin", "__builtin_debugtrap", false),
    ("__has_builtin", "__builtin_dynamic_object_size", true),
    ("__has_builtin", "__builtin_expect", true),
    ("__has_builtin", "__builtin_fclose", false),
    ("__has_builtin", "__builtin_ffs", true),
    ("__has_builtin", "__builtin_FILE", true),
    ("__has_builtin", "__builtin_frame_address", true),
    ("__has_builtin", "__builtin_ia32_pause", true),
    ("__has_builtin", "__builtin_is_constant_evaluated", false),
    ("__has_builtin", "__builtin_is_corresponding_member", false),
    ("__has_builtin", "__builtin_is_pointer_interconvertible_with_class", false),
    ("__has_builtin", "__builtin_memcpy", true),
    ("__has_builtin", "__builtin_memset", true),
    ("__has_builtin", "__builtin_mul_overflow", true),
    ("__has_builtin", "__builtin_object_size", true),
    ("__has_builtin", "__builtin_operator_new", false),
    ("__has_builtin", "__builtin_parity", true),
    ("__has_builtin", "__builtin_popcount", true),
    ("__has_builtin", "__builtin_popcountll", true),
    ("__has_builtin", "__builtin_prefetch", true),
    ("__has_builtin", "__builtin_return_address", true),
    ("__has_builtin", "__builtin_setjmp", true),
    ("__has_builtin", "__builtin_shuffle", true),
    ("__has_builtin", "__builtin_shufflevector", true),
    ("__has_builtin", "__builtin_source_location", false),
    ("__has_builtin", "__builtin_sprintf", true),
    ("__has_builtin", "__builtin_stdc_bit_ceil", false),
    ("__has_builtin", "__builtin_stdc_bit_floor", false),
    ("__has_builtin", "__builtin_stdc_bit_width", false),
    ("__has_builtin", "__builtin_stdc_count_ones", false),
    ("__has_builtin", "__builtin_stdc_count_zeros", false),
    ("__has_builtin", "__builtin_stdc_first_leading_one", false),
    ("__has_builtin", "__builtin_stdc_first_leading_zero", false),
    ("__has_builtin", "__builtin_stdc_first_trailing_one", false),
    ("__has_builtin", "__builtin_stdc_first_trailing_zero", false),
    ("__has_builtin", "__builtin_stdc_has_single_bit", false),
    ("__has_builtin", "__builtin_stdc_leading_ones", false),
    ("__has_builtin", "__builtin_stdc_leading_zeros", false),
    ("__has_builtin", "__builtin_stdc_trailing_ones", false),
    ("__has_builtin", "__builtin_stdc_trailing_zeros", false),
    ("__has_builtin", "__builtin_sub_overflow", true),
    ("__has_builtin", "__builtin_toupper", true),
    ("__has_builtin", "__builtin_trap", true),
    ("__has_builtin", "__builtin_types_compatible_p", true),
    ("__has_builtin", "__builtin_unreachable", true),
    ("__has_builtin", "__builtin_va_arg_pack", true),
];

/// The answer, and whether the table had one.
///
/// `None` means **this list does not cover the name**, which is a different fact from `Some(false)`
/// and is why the return type is not a `bool`.
pub fn answer(query: &str, name: &str) -> Option<bool> {
    TABLE
        .iter()
        .find(|(q, n, _)| *q == query && *n == name)
        .map(|(_, _, supported)| *supported)
}

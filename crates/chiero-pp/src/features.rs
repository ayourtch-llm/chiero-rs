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
//! not conjured for the ones it lacks. Forty-six of the entries here are **zero**, and they are
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
pub const TABLE: &[(&str, &str, u32)] = &[
    ("__has_attribute", "access", 1),
    ("__has_attribute", "__aligned__", 1),
    ("__has_attribute", "aligned", 1),
    ("__has_attribute", "__alloc_align__", 1),
    ("__has_attribute", "alloc_align", 1),
    ("__has_attribute", "alloc_size", 1),
    ("__has_attribute", "__always_inline__", 1),
    ("__has_attribute", "always_inline", 1),
    ("__has_attribute", "__artificial__", 1),
    ("__has_attribute", "artificial", 1),
    ("__has_attribute", "__assume__", 1),
    ("__has_attribute", "__attribute_deprecated_with_message__", 0),
    ("__has_attribute", "__bases", 0),
    ("__has_attribute", "c_generic_selections", 0),
    ("__has_attribute", "__cleanup__", 1),
    ("__has_attribute", "cleanup", 1),
    ("__has_attribute", "__cold__", 1),
    ("__has_attribute", "cold", 1),
    ("__has_attribute", "__const__", 1),
    ("__has_attribute", "const", 1),
    ("__has_attribute", "__constructor__", 1),
    ("__has_attribute", "constructor", 1),
    ("__has_attribute", "__copy__", 1),
    ("__has_attribute", "copy", 1),
    ("__has_attribute", "__deprecated__", 201904),
    ("__has_attribute", "deprecated", 201904),
    ("__has_attribute", "__destructor__", 1),
    ("__has_attribute", "destructor", 1),
    ("__has_attribute", "__direct_bases", 0),
    ("__has_attribute", "enable_if", 0),
    ("__has_attribute", "__fallthrough__", 201910),
    ("__has_attribute", "fallthrough", 201910),
    ("__has_attribute", "__format__", 1),
    ("__has_attribute", "format", 1),
    ("__has_attribute", "__format_arg__", 1),
    ("__has_attribute", "__has_unique_object_representations", 0),
    ("__has_attribute", "hot", 1),
    ("__has_attribute", "__indirect_return__", 1),
    ("__has_attribute", "__init_priority__", 0),
    ("__has_attribute", "__is_convertible", 0),
    ("__has_attribute", "__is_layout_compatible", 0),
    ("__has_attribute", "__is_nothrow_convertible", 0),
    ("__has_attribute", "__is_pointer_interconvertible_base_of", 0),
    ("__has_attribute", "leaf", 1),
    ("__has_attribute", "__make_integer_seq", 0),
    ("__has_attribute", "__malloc__", 1),
    ("__has_attribute", "malloc", 1),
    ("__has_attribute", "__may_alias__", 1),
    ("__has_attribute", "may_alias", 1),
    ("__has_attribute", "minsize", 0),
    ("__has_attribute", "__mode__", 1),
    ("__has_attribute", "mode", 1),
    ("__has_attribute", "noclone", 1),
    ("__has_attribute", "nodebug", 0),
    ("__has_attribute", "__noinline__", 1),
    ("__has_attribute", "noinline", 1),
    ("__has_attribute", "__nonnull__", 1),
    ("__has_attribute", "nonnull", 1),
    ("__has_attribute", "noplt", 1),
    ("__has_attribute", "no_profile_instrument_function", 1),
    ("__has_attribute", "__noreturn__", 202202),
    ("__has_attribute", "noreturn", 202202),
    ("__has_attribute", "no_sanitize", 1),
    ("__has_attribute", "__nothrow__", 1),
    ("__has_attribute", "nothrow", 1),
    ("__has_attribute", "__packed__", 1),
    ("__has_attribute", "packed", 1),
    ("__has_attribute", "preferred_type", 0),
    ("__has_attribute", "__pure__", 1),
    ("__has_attribute", "pure", 1),
    ("__has_attribute", "__reference_constructs_from_temporary", 0),
    ("__has_attribute", "__reference_converts_from_temporary", 0),
    ("__has_attribute", "__remove_cv", 0),
    ("__has_attribute", "__remove_cvref", 0),
    ("__has_attribute", "__remove_reference", 0),
    ("__has_attribute", "__returns_nonnull__", 1),
    ("__has_attribute", "returns_nonnull", 1),
    ("__has_attribute", "returns_twice", 1),
    ("__has_attribute", "__section__", 1),
    ("__has_attribute", "section", 1),
    ("__has_attribute", "sentinel", 1),
    ("__has_attribute", "symver", 1),
    ("__has_attribute", "__transparent_union__", 1),
    ("__has_attribute", "transparent_union", 1),
    ("__has_attribute", "__unused__", 1),
    ("__has_attribute", "unused", 1),
    ("__has_attribute", "__used__", 1),
    ("__has_attribute", "used", 1),
    ("__has_attribute", "__vector_size__", 1),
    ("__has_attribute", "vector_size", 1),
    ("__has_attribute", "__visibility__", 1),
    ("__has_attribute", "visibility", 1),
    ("__has_attribute", "__warn_unused_result__", 1),
    ("__has_attribute", "warn_unused_result", 1),
    ("__has_attribute", "__weak__", 1),
    ("__has_attribute", "weak", 1),
    ("__has_builtin", "__builtin_add_overflow", 1),
    ("__has_builtin", "__builtin_alloca", 1),
    ("__has_builtin", "__builtin_assume_aligned", 1),
    ("__has_builtin", "__builtin_bit_cast", 0),
    ("__has_builtin", "__builtin_bitreverse16", 0),
    ("__has_builtin", "__builtin_bitreverse32", 0),
    ("__has_builtin", "__builtin_bitreverse64", 0),
    ("__has_builtin", "__builtin_bitreverse8", 0),
    ("__has_builtin", "__builtin_bswap128", 1),
    ("__has_builtin", "__builtin_bswap16", 1),
    ("__has_builtin", "__builtin_bswap32", 1),
    ("__has_builtin", "__builtin_bswap64", 1),
    ("__has_builtin", "__builtin_choose_expr", 1),
    ("__has_builtin", "__builtin_clear_padding", 1),
    ("__has_builtin", "__builtin_clz", 1),
    ("__has_builtin", "__builtin_clzl", 1),
    ("__has_builtin", "__builtin_clzll", 1),
    ("__has_builtin", "__builtin_constant_p", 1),
    ("__has_builtin", "__builtin_ctz", 1),
    ("__has_builtin", "__builtin_ctzl", 1),
    ("__has_builtin", "__builtin_ctzll", 1),
    ("__has_builtin", "__builtin_debugtrap", 0),
    ("__has_builtin", "__builtin_dynamic_object_size", 1),
    ("__has_builtin", "__builtin_expect", 1),
    ("__has_builtin", "__builtin_fclose", 0),
    ("__has_builtin", "__builtin_ffs", 1),
    ("__has_builtin", "__builtin_FILE", 1),
    ("__has_builtin", "__builtin_frame_address", 1),
    ("__has_builtin", "__builtin_ia32_pause", 1),
    ("__has_builtin", "__builtin_is_constant_evaluated", 0),
    ("__has_builtin", "__builtin_is_corresponding_member", 0),
    ("__has_builtin", "__builtin_is_pointer_interconvertible_with_class", 0),
    ("__has_builtin", "__builtin_memcpy", 1),
    ("__has_builtin", "__builtin_memset", 1),
    ("__has_builtin", "__builtin_mul_overflow", 1),
    ("__has_builtin", "__builtin_object_size", 1),
    ("__has_builtin", "__builtin_operator_new", 0),
    ("__has_builtin", "__builtin_parity", 1),
    ("__has_builtin", "__builtin_popcount", 1),
    ("__has_builtin", "__builtin_popcountll", 1),
    ("__has_builtin", "__builtin_prefetch", 1),
    ("__has_builtin", "__builtin_return_address", 1),
    ("__has_builtin", "__builtin_setjmp", 1),
    ("__has_builtin", "__builtin_shuffle", 1),
    ("__has_builtin", "__builtin_shufflevector", 1),
    ("__has_builtin", "__builtin_source_location", 0),
    ("__has_builtin", "__builtin_sprintf", 1),
    ("__has_builtin", "__builtin_stdc_bit_ceil", 0),
    ("__has_builtin", "__builtin_stdc_bit_floor", 0),
    ("__has_builtin", "__builtin_stdc_bit_width", 0),
    ("__has_builtin", "__builtin_stdc_count_ones", 0),
    ("__has_builtin", "__builtin_stdc_count_zeros", 0),
    ("__has_builtin", "__builtin_stdc_first_leading_one", 0),
    ("__has_builtin", "__builtin_stdc_first_leading_zero", 0),
    ("__has_builtin", "__builtin_stdc_first_trailing_one", 0),
    ("__has_builtin", "__builtin_stdc_first_trailing_zero", 0),
    ("__has_builtin", "__builtin_stdc_has_single_bit", 0),
    ("__has_builtin", "__builtin_stdc_leading_ones", 0),
    ("__has_builtin", "__builtin_stdc_leading_zeros", 0),
    ("__has_builtin", "__builtin_stdc_trailing_ones", 0),
    ("__has_builtin", "__builtin_stdc_trailing_zeros", 0),
    ("__has_builtin", "__builtin_sub_overflow", 1),
    ("__has_builtin", "__builtin_toupper", 1),
    ("__has_builtin", "__builtin_trap", 1),
    ("__has_builtin", "__builtin_types_compatible_p", 1),
    ("__has_builtin", "__builtin_unreachable", 1),
    ("__has_builtin", "__builtin_va_arg_pack", 1),
    ("__has_c_attribute", "_Noreturn", 202202),
    ("__has_c_attribute", "__deprecated__", 201904),
    ("__has_c_attribute", "__fallthrough__", 201910),
    ("__has_c_attribute", "__maybe_unused__", 202106),
    ("__has_c_attribute", "__nodiscard__", 202003),
    ("__has_c_attribute", "__noreturn__", 202202),
    ("__has_c_attribute", "__reproducible__", 0),
    ("__has_c_attribute", "__unsequenced__", 0),
    ("__has_c_attribute", "deprecated", 201904),
    ("__has_c_attribute", "fallthrough", 201910),
    ("__has_c_attribute", "maybe_unused", 202106),
    ("__has_c_attribute", "nodiscard", 202003),
    ("__has_c_attribute", "noreturn", 202202),
    ("__has_c_attribute", "reproducible", 0),
    ("__has_c_attribute", "unsequenced", 0),
];

/// The answer, and whether the table had one.
///
/// `None` means **this list does not cover the name**, which is a different fact from `Some(0)`
/// and is why the return type is not a bare number.
///
/// ⚠️ **The value is a number, not a truth.** `__has_attribute` and `__has_builtin` answer 1 or
/// 0, but `__has_c_attribute` answers the C standard's *version* for the attribute —
/// `deprecated` is `201904` under gcc 13 — and a `bool` here could only have recorded that it
/// was non-zero. The table would then have claimed 1 where the compiler says 201904, and the
/// differential test, which compared truthiness, would have agreed with it.
pub fn answer(query: &str, name: &str) -> Option<u32> {
    TABLE
        .iter()
        .find(|(q, n, _)| *q == query && *n == name)
        .map(|(_, _, value)| *value)
}

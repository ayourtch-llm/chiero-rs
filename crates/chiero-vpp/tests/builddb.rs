//! Covers: 060 contract 1 — "`compile_commands.json` from a real VPP build parses, and every TU
//! yields a `ConfigId` and a resolved include path set."
//!
//! **This unblocks a stale blocker, not a new design.** 060 §1 says "*The VPP tree at
//! `/home/ubuntu/vpp` is not built yet — no `compile_commands.json` exists*", which was true when
//! written and is false now. The database does not exist as a *file* even today, but
//! `ninja -C <build> -t compdb` emits it on stdout in 90 ms, so the ingest reads text, not a path.
//!
//! **Every shape asserted below was measured in VPP's own database first** (6235 entries, 2226 of
//! them C), so these fixtures are miniatures of real rows rather than invented ones:
//!
//! | measured in VPP | fixture that pins it |
//! |---|---|
//! | 259 of 2226 entries have a **relative** `file` | `a_relative_file_resolves_against_its_directory` |
//! | 275 sources appear under **more than one** TU (max 9) | `one_source_can_produce_several_translation_units` |
//! | 5495 `-D` and 7857 `-I`, and **nothing else** configuration-bearing | `defines_and_include_paths_are_extracted_from_the_command` |
//! | 2226 TUs carry only **418** distinct (defines, includes) | `two_units_sharing_a_configuration_share_a_config_id` |

use chiero_vpp::builddb::BuildDb;
use std::path::{Path, PathBuf};

/// One entry, written the way `ninja -t compdb` writes them: a `command` string with the
/// driver in front, and `-D`/`-I` mixed in among flags that say nothing about configuration.
fn entry(file: &str, dir: &str, flags: &str, object: &str) -> String {
    format!(
        r#"{{"directory": "{dir}", "command": "/usr/lib/ccache/clang {flags} -o {object} -c {file}", "file": "{file}", "output": "{object}"}}"#
    )
}

fn db(entries: &[String]) -> BuildDb {
    BuildDb::parse(&format!("[{}]", entries.join(",\n"))).expect("fixture parses")
}

/// **259 of VPP's 2226 C entries name their source relatively** (`CMakeFiles/plugins/nat/…`),
/// and the rest absolutely. A reader that takes `file` at face value looks for a quarter of the
/// corpus in whatever directory it happens to be running from.
///
/// The negative half matters as much: an already-absolute `file` must *not* be re-rooted.
#[test]
fn a_relative_file_resolves_against_its_directory() {
    let d = db(&[
        entry("CMakeFiles/gen/api.c", "/build/vpp", "-I/src", "a.o"),
        entry(
            "/home/ubuntu/vpp/src/vnet/ip.c",
            "/build/vpp",
            "-I/src",
            "b.o",
        ),
    ]);
    assert_eq!(
        d.units().iter().map(|u| u.src.clone()).collect::<Vec<_>>(),
        vec![
            PathBuf::from("/build/vpp/CMakeFiles/gen/api.c"),
            PathBuf::from("/home/ubuntu/vpp/src/vnet/ip.c"),
        ],
        "a relative `file` is joined to `directory`; an absolute one is left alone"
    );
}

/// 060 §1.1: "**The source→TU mapping is 1:N, not 1:1.** Every index keyed by file path is
/// wrong." Measured: 275 of VPP's 1710 distinct C sources compile more than once, the worst
/// nine times over.
///
/// So the lookup returns *all* of them, and the test would fail on any implementation that
/// keyed a map by path — the classic shape, where the last row silently wins.
#[test]
fn one_source_can_produce_several_translation_units() {
    let d = db(&[
        entry(
            "/src/aes.c",
            "/b",
            "-DCLIB_MARCH_VARIANT=x86_64_v3 -I/src",
            "v3.o",
        ),
        entry(
            "/src/aes.c",
            "/b",
            "-DCLIB_MARCH_VARIANT=scalar -I/src",
            "sc.o",
        ),
        entry("/src/other.c", "/b", "-I/src", "o.o"),
    ]);
    let aes: Vec<_> = d.units_for(Path::new("/src/aes.c")).collect();
    assert_eq!(aes.len(), 2, "both variants of one source are kept");
    assert_ne!(
        aes[0].config, aes[1].config,
        "and they are different configurations, because the define differs"
    );
    assert_eq!(
        aes.iter().map(|u| u.object.clone()).collect::<Vec<_>>(),
        vec![PathBuf::from("v3.o"), PathBuf::from("sc.o")],
        "each keeps its own object, so a finding can name the variant it came from"
    );
    assert_eq!(d.units_for(Path::new("/src/nothing.c")).count(), 0);
}

/// The extraction itself: joined (`-I/src`) and separated (`-I /src`) spellings, a define with
/// a value, and a **bare define**, which C says is `1` — not the empty string, which would make
/// `#if BARE` false and silently delete code.
///
/// The `-Wall -g -MD -MF …` around them are noise on purpose: they are 2000+ of the tokens in a
/// real VPP command line, and an extractor that swallowed them would report nonsense.
#[test]
fn defines_and_include_paths_are_extracted_from_the_command() {
    let d = db(&[entry(
        "/src/x.c",
        "/b",
        "-DHAVE_FCNTL64 -D_FORTIFY_SOURCE=2 -I/home/ubuntu/vpp/src -I /b/CMakeFiles -Wall -g -MD",
        "x.o",
    )]);
    let u = &d.units()[0];
    assert_eq!(
        u.defines,
        vec![
            ("HAVE_FCNTL64".to_string(), "1".to_string()),
            ("_FORTIFY_SOURCE".to_string(), "2".to_string()),
        ],
        "a bare -D is 1, per C11 6.10.3p9's `#define NAME 1`"
    );
    assert_eq!(
        u.include_paths,
        vec![
            PathBuf::from("/home/ubuntu/vpp/src"),
            PathBuf::from("/b/CMakeFiles"),
        ],
        "-I<path> and -I <path> are the same flag, and order is search order"
    );
}

/// **What a `ConfigId` is for.** 012 §3.3: the flags "determine the `ConfigId`, which determines
/// which `#if` branches exist, which determines layout, which determines every offset in the
/// analysis." So the id must be a function of *exactly* the configuration-bearing flags:
/// hashing the whole command line would make every TU unique and buy nothing.
///
/// Measured on VPP: **2226 C TUs carry 418 distinct configurations**, a 5.3× collapse. That
/// number is the contract's whole value, and it only exists if `-o`, `-MF`, `-Wall` and friends
/// are excluded. Both directions are asserted, because an id that ignored `-D` too would also
/// pass the first half.
#[test]
fn two_units_sharing_a_configuration_share_a_config_id() {
    let d = db(&[
        entry("/src/a.c", "/b", "-DFOO=1 -I/src -Wall -g", "a.o"),
        // Same -D and -I; everything else differs, including the source itself.
        entry(
            "/src/b.c",
            "/b",
            "-DFOO=1 -I/src -Werror -O3 -MD -MF b.d",
            "b.o",
        ),
        // One define differs.
        entry("/src/c.c", "/b", "-DFOO=2 -I/src -Wall -g", "c.o"),
        // One include path differs.
        entry("/src/d.c", "/b", "-DFOO=1 -I/other -Wall -g", "d.o"),
    ]);
    let [a, b, c, e] =
        <[_; 4]>::try_from(d.units().iter().map(|u| u.config).collect::<Vec<_>>()).unwrap();
    assert_eq!(
        a, b,
        "warning, debug and output flags do not change a ConfigId"
    );
    assert_ne!(a, c, "but a differing -D does");
    assert_ne!(
        a, e,
        "and so does a differing -I, since it changes what a #include finds"
    );
    assert_ne!(
        a,
        chiero_pp::ConfigId::default(),
        "060 contract 1: every TU yields a ConfigId — a default one names nothing"
    );
    assert_eq!(d.distinct_configs(), 3);
}

/// The ingest hands the preprocessor a ready-made `Config`, which is the only reason any of the
/// above is worth extracting. Anything less and every caller re-derives it differently.
#[test]
fn a_unit_yields_a_preprocessor_config_carrying_its_own_id() {
    let d = db(&[entry("/src/a.c", "/b", "-DFOO -I/src", "a.o")]);
    let u = &d.units()[0];
    let cfg = u.pp_config();
    assert_eq!(cfg.id, u.config);
    assert_eq!(cfg.defines, vec![("FOO".to_string(), "1".to_string())]);
    assert_eq!(cfg.include_paths, vec![PathBuf::from("/src")]);
}

/// `compile_commands.json` also has an `arguments` array form (CMake emits it; `ninja -t compdb`
/// emits `command`). Both are the format, so both parse — and the shell form must respect quotes,
/// or a `-DFOO="a b"` splits into two flags and the define is silently wrong.
#[test]
fn both_the_command_string_and_the_arguments_array_are_accepted() {
    let d = BuildDb::parse(
        r#"[
        {"directory":"/b","arguments":["clang","-DA=1","-I","/src","-c","/src/a.c"],"file":"/src/a.c","output":"a.o"},
        {"directory":"/b","command":"clang -DB=\"x y\" -I/src -c /src/b.c","file":"/src/b.c","output":"b.o"}
        ]"#,
    )
    .unwrap();
    assert_eq!(d.units()[0].defines, vec![("A".into(), "1".into())]);
    assert_eq!(d.units()[0].include_paths, vec![PathBuf::from("/src")]);
    assert_eq!(
        d.units()[1].defines,
        vec![("B".to_string(), "x y".to_string())],
        "a quoted define value survives tokenization as one value"
    );
}

/// Non-C entries (`.cpp`, `.S`, and the 4009 non-C rows in VPP's database) are not the
/// frontend's business, and a caller that wants only C should not have to filter by extension
/// itself — every caller would do it slightly differently.
#[test]
fn c_units_are_separable_from_the_rest_of_the_database() {
    let d = db(&[
        entry("/src/a.c", "/b", "-I/src", "a.o"),
        entry("/src/b.cpp", "/b", "-I/src", "b.o"),
        entry("/src/c.S", "/b", "-I/src", "c.o"),
    ]);
    assert_eq!(d.units().len(), 3, "the database is kept whole");
    assert_eq!(
        d.c_units().map(|u| u.src.clone()).collect::<Vec<_>>(),
        vec![PathBuf::from("/src/a.c")]
    );
}

/// Malformed input is an error, not a panic and not an empty database. **An empty database is
/// the dangerous answer**: every downstream metric would report "0 TUs failed" and look green.
#[test]
fn a_database_that_is_not_one_is_an_error_rather_than_an_empty_answer() {
    assert!(BuildDb::parse("not json").is_err());
    assert!(
        BuildDb::parse(r#"{"file":"a.c"}"#).is_err(),
        "the top level is an array"
    );
    assert!(
        BuildDb::parse(r#"[{"directory":"/b","command":"clang -c a.c"}]"#).is_err(),
        "an entry with no `file` names no translation unit"
    );
    assert!(
        BuildDb::parse("[]").unwrap().units().is_empty(),
        "but empty *is* empty"
    );
}

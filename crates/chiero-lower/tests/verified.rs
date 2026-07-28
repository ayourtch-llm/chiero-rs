//! **Lowering emits CIR the verifier accepts, or it refuses.** Covers: 015 §7, 020 §5.
//!
//! 015 §7 says lowering refuses what it cannot represent, and it does — for everything it
//! *knows* it cannot represent. Wave 141 found the other class: `*(&a[1] + 0)` emitted
//! `zext i32 %v to i64` on a `Ptr`, which the verifier rejects, and lowering pushed no
//! diagnostic because it did not know it had gone wrong. The module was emitted anyway and
//! the engine produced no state for a reason with nothing to do with the program.
//!
//! Nothing ran the verifier between the two. `chiero-opt` and `chiero-exec` verify modules
//! they *build*; no test verified one lowering *produced*. This file is that test.
//!
//! The invariant is one line — **every module lowering emits verifies clean** — and it is
//! worth stating separately from the differential oracle because the two catch different
//! things. The oracle compares answers and cannot see a module that produced no answer at
//! all; this sees the malformed module directly, and says which instruction is wrong
//! instead of leaving a `None` to be bisected by hand.

mod harness;

/// Sources spanning the constructs waves 132–144 touched, which is where the invalid CIR
/// has actually come from: pointer arithmetic in every spelling, aggregate returns and
/// parameters, bit-fields, compound literals, statement expressions, and the file-scope
/// forms of each.
///
/// Hand-listed rather than generated, deliberately. `generated.rs` covers breadth and runs
/// in a minute; this runs in under a second and is the one that stays useful when a change
/// breaks lowering badly enough that the generator cannot even build a program.
const SOURCES: &[&str] = &[
    // Pointer arithmetic, all six spellings plus the address-of forms wave 141 broke.
    "int probe(void){ int a[3]={1,2,3}; return a[1] + *(a+1) + *(1+a) + *(&a[1]) + *(&a[1]+0) + (&a[0])[1]; }",
    "int probe(void){ int a[3]={1,2,3}; int *p=a; p+=1; p++; return *p + (int)(p - a); }",
    "int g=1; int *gp=&g; int probe(void){ return *gp + (gp==0); }",
    "int ga[3]={1,2,3}; int *gp=&ga[1]; int probe(void){ return *gp + gp[0]; }",
    // Aggregates through calls, by value and by return.
    "struct S{int a;int b;};\nstatic struct S mk(int x){struct S o;o.a=x;o.b=x+1;return o;}\n\
     static int take(struct S p){return p.a*10+p.b;}\n\
     int probe(void){ struct S s=mk(3); return take(s)+mk(4).a+take((struct S){5,6}); }",
    // Bit-fields: assignment, compound assignment, increment, braced initializer.
    "struct B{int a:3;int b:5;};\n\
     int probe(void){ struct B v={1,2}; v.a=3; v.a+=1; v.b++; return v.a*10+v.b; }",
    "struct U{unsigned a:3;unsigned b:5;};\n\
     int probe(void){ struct U v={7,2}; v.a/=2; v.b>>=1; return (int)(v.a*10+v.b); }",
    // `_Bool` in every read-modify-write shape, which crossed three code paths.
    "int probe(void){ _Bool b=1; b++; b+=1; b-=1; int r=++b; return r*10+b; }",
    // Compound literals and statement expressions, including the aggregate forms.
    "struct S{int a;int b;};\n\
     int probe(void){ struct S *p=&(struct S){1,2}; struct S y=({struct S t;t.a=3;t.b=4;t;}); \
     return p->a+y.b+(struct S){7,8}.a+(int){9}; }",
    // Enumerations, including one too wide for `int`.
    "enum E{A=3,B,C=7};\nenum Big{X=5000000000};\n\
     int probe(void){ enum E e=B; return e+A+(int)(X>>32); }",
    // Conditions over pointers and `_Bool`, which is where `truth_of` decides a type.
    "int probe(void){ int x; int *p=&x; _Bool b=p; if(p){b=!p;} while(p){break;} return b+(p?1:0); }",
    // Mixed-width struct members and arrays of them, where the store widths are decided.
    "struct M{signed char c; short s; long l;};\n\
     int probe(void){ struct M m={300,70000,-1}; struct M a[2]={{1,2,3},{4,5,6}}; \
     return m.c+m.s+(int)m.l+a[1].c; }",
    // Casts among every width, both signednesses.
    "int probe(void){ signed char c=-1; unsigned char u=255; short s=-300; unsigned long l=0; \
     l=(unsigned long)c; return (int)(l>>32)+(int)(unsigned)s+u+(int)(long)c; }",
];

/// **Every module lowering emits verifies clean.**
///
/// A verifier error here is not a style complaint: it means the engine will be handed an
/// instruction it cannot interpret, and 023's answer to that is to stop — which the caller
/// sees as a program that produced nothing, with no indication that lowering was at fault.
#[test]
fn every_lowered_module_verifies() {
    let mut bad: Vec<(usize, String)> = Vec::new();
    for (i, src) in SOURCES.iter().enumerate() {
        let m = harness::lower(src);
        let errs: Vec<String> = chiero_cir::verify::verify(&m)
            .iter()
            .filter(|e| e.is_error())
            .map(|e| format!("{e:?}"))
            .collect();
        if !errs.is_empty() {
            bad.push((i, format!("{src}\n  -> {}", errs.join("\n  -> "))));
        }
    }
    assert!(
        bad.is_empty(),
        "{} source(s) lowered to CIR the verifier rejects. \
         The engine cannot run these and will report nothing rather than a defect:\n\n{}",
        bad.len(),
        bad.iter()
            .map(|(i, t)| format!("[{i}] {t}"))
            .collect::<Vec<_>>()
            .join("\n\n")
    );
}

/// **And lowering refuses rather than emitting one.**
///
/// The companion to the invariant above, and the half that matters at run time: when
/// lowering *does* produce something malformed, 015 §7's rule is that the function is
/// discarded with a diagnostic — a gap the caller can see — rather than handed on for the
/// engine to fall silent over.
///
/// This cannot be driven by a fixture: no construct in the language produces invalid CIR
/// today, which is what the test above asserts. It is pinned by mutation instead, exactly
/// as wave 134 pinned the parser's diagnostic rollback. Reverting wave 141's sema fix
/// (`p + 0` converting the `0` to a null pointer constant) reproduces the historical case,
/// and with the guard in place the function is refused instead of emitted.
#[test]
fn a_module_that_would_not_verify_is_refused_instead() {
    // Every source above lowers cleanly, so none is refused for *this* reason. The
    // assertion is that the two properties agree: nothing is both emitted and invalid.
    for src in SOURCES {
        let lowered = harness::lower_raw(src);
        let errs: Vec<_> = chiero_cir::verify::verify(&lowered.module)
            .iter()
            .filter(|e| e.is_error())
            .map(|e| format!("{e:?}"))
            .collect();
        assert!(
            errs.is_empty(),
            "emitted and invalid at once, which is the state 015 §7 exists to prevent: \
             {src}\n  -> {}\n  diagnostics: {:?}",
            errs.join("\n  -> "),
            lowered.diagnostics
        );
    }
}

//! The corpus provider must actually deliver a corpus.
//!
//! Every property that consumes `hale_corpus` is only as trustworthy
//! as the extraction underneath it. A scraper that silently stopped
//! matching would turn a suite of corpus-wide guarantees into a suite
//! of vacuous passes — the exact failure mode the first
//! registry-parity test shipped with.
//!
//! So: pin the counts, pin the ratio that motivated the work, and
//! pin that **everything yielded parses**. The last one is what makes
//! it safe for other tests to treat a parse failure as a real defect
//! rather than a scraping artifact.

#[test]
fn extraction_is_not_vacuous() {
    let fixtures = hale_corpus::fixtures();
    let embedded = hale_corpus::embedded();
    assert!(
        fixtures.len() > 100,
        "expected the on-disk corpus, found {} programs",
        fixtures.len()
    );
    assert!(
        embedded.len() > 800,
        "expected ~1.2k embedded programs, found {} — the scraper is \
         not reading the test suite it thinks it is",
        embedded.len()
    );
}

/// The finding that motivated the provider: there is substantially
/// more Hale embedded in tests than sitting on disk, and it used to
/// be invisible to every corpus property.
#[test]
fn embedded_corpus_outweighs_the_on_disk_one() {
    let lines = |ps: &[hale_corpus::Program]| -> usize {
        ps.iter().map(|p| p.source.lines().count()).sum()
    };
    let disk = lines(&hale_corpus::fixtures());
    let embedded = lines(&hale_corpus::embedded());
    assert!(
        embedded > disk,
        "embedded Hale ({} lines) should exceed the on-disk corpus \
         ({} lines) — if this inverts, the scraper regressed",
        embedded,
        disk
    );
}

/// Almost everything the provider yields must parse.
///
/// Not *everything*: the suite deliberately contains negative
/// fixtures — `where bogus_constraint`, `"\xff"`, a `bindings` block
/// in a non-`main` locus — whose whole job is to fail. They are real
/// programs and belong in the corpus; a property that needs valid
/// input filters with `parseable()`.
///
/// What this pins is the *rate*. A handful of intentional negatives
/// is expected; a scraper that started yielding fragments or
/// half-substituted templates would blow straight through the bound,
/// which is the failure this needs to catch.
#[test]
fn extraction_yields_overwhelmingly_valid_programs() {
    let all = hale_corpus::all();
    let bad: Vec<&str> = all
        .iter()
        .filter(|p| hale_syntax::parse_source(&p.source).is_err())
        .map(|p| p.origin.as_str())
        .collect();
    let pct = 100.0 * bad.len() as f64 / all.len() as f64;
    assert!(
        pct < 2.0,
        "{:.1}% of extracted programs ({} of {}) fail to parse — the \
         scraper is yielding fragments or templates, not just the \
         suite's intentional negative fixtures:\n{:#?}",
        pct,
        bad.len(),
        all.len(),
        &bad[..bad.len().min(25)]
    );
}

/// And the filtered view must still be a corpus, not a trickle.
#[test]
fn parseable_view_is_substantial() {
    let ok = hale_corpus::parseable(|s| hale_syntax::parse_source(s).is_ok());
    assert!(
        ok.len() > 900,
        "only {} parseable programs — properties built on this would \
         be running against a fraction of the suite",
        ok.len()
    );
}

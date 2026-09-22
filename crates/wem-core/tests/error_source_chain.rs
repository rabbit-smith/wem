//! The public error chain must reach the innermost cause.
//!
//! `EncoderError` wraps `InternalError`, which wraps the kernel-stage error
//! (`ProfileError`, `AnalysisError`, `PacketError`, `ContainerError`). With
//! empty `Error` impls every level reported `source() == None`, so a caller
//! holding an `EncoderError` could not get from "encoder fault" to the stage
//! failure that produced it. These suites pin the chain: the layers are
//! walked through `source()`, and the leaf is the concrete inner error.

use std::error::Error;

use wem_analysis::config::AnalysisError;
use wem_core::error::{EncoderError, InternalError};
use wem_core::Encoder;
use wem_profiles::error::ProfileError;

mod common;

use common::fixture_selection;

/// The message chain reachable from `error` through `source()`, head first.
fn chain(error: &(dyn Error + 'static)) -> Vec<String> {
    let mut messages = vec![error.to_string()];
    let mut current = error.source();
    while let Some(cause) = current {
        messages.push(cause.to_string());
        current = cause.source();
    }
    messages
}

#[test]
fn a_real_selection_fault_reaches_the_profile_error() {
    // A quality request on the 6ch/44100 configuration: that profile ships no
    // quality-curves resource, so profile assembly is where the kernel fails.
    let error = Encoder::new_with_quality(fixture_selection(), Some(0.5))
        .expect_err("the 6ch/44100 profile has no quality-curves resource");

    let internal = error
        .source()
        .expect("EncoderError::Internal must expose its InternalError")
        .downcast_ref::<InternalError>()
        .unwrap_or_else(|| panic!("EncoderError::source() is not the InternalError: {error:?}"));
    assert!(
        matches!(internal, InternalError::Profile(_)),
        "expected an InternalError::Profile fault, got {internal}"
    );

    let profile = internal
        .source()
        .expect("InternalError::Profile must expose its ProfileError")
        .downcast_ref::<ProfileError>()
        .unwrap_or_else(|| panic!("InternalError::source() is not the ProfileError: {internal:?}"));
    assert!(
        matches!(profile, ProfileError::QualityCurvesResourceMissing { .. }),
        "unexpected leaf profile error: {profile}"
    );
    assert!(
        profile.source().is_none(),
        "a leaf profile error must not invent a cause: {profile}"
    );

    let messages = chain(&error);
    assert_eq!(
        messages.len(),
        3,
        "expected encoder -> internal -> profile, got {messages:?}"
    );
    assert!(
        messages[0].starts_with("encoder fault: "),
        "the head message must stay the EncoderError diagnostic: {messages:?}"
    );
    assert!(
        messages[1].contains("quality-curves"),
        "the InternalError message must name the stage fault: {messages:?}"
    );
    assert!(
        messages[2].contains("quality-curves"),
        "the innermost cause must be reachable through source(): {messages:?}"
    );
}

#[test]
fn every_wrapping_layer_appears_in_the_chain() {
    let innermost = AnalysisError::PsyCurveLengthMismatch {
        want: 1024,
        got: 512,
    };
    let error = EncoderError::Internal(InternalError::Profile(ProfileError::Analysis(
        innermost.clone(),
    )));

    let messages = chain(&error);
    assert_eq!(
        messages.len(),
        4,
        "expected encoder -> internal -> profile -> analysis, got {messages:?}"
    );
    assert_eq!(
        messages[3],
        innermost.to_string(),
        "the analysis error's own Display text must end the chain"
    );
    assert!(
        messages[3].contains("1024") && messages[3].contains("512"),
        "the analysis message must carry its observed values: {messages:?}"
    );
    assert!(
        !messages[3].contains("PsyCurveLengthMismatch"),
        "AnalysisError's Display must be a message, not its Debug rendering: {messages:?}"
    );
    // The profile layer's own wording for this variant is untouched by this
    // change (it still prints the analysis Debug rendering); only the cause
    // chain is new.
    assert!(
        messages[2].starts_with("analysis error: "),
        "the profile layer must keep its message prefix: {messages:?}"
    );
}

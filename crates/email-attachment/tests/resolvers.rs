use email_attachment::{
    AttachmentResolver, FallbackResolver, MapResolver, ResolveErrorKind, ResolvedAttachment,
    SchemeDispatch, SchemeRouter,
};
use email_message::AttachmentReference;

#[test]
fn resolved_attachment_converts_from_bytes() {
    let resolved = ResolvedAttachment::from(b"pdf bytes".to_vec());

    assert_eq!(resolved, ResolvedAttachment::new(b"pdf bytes"));
    assert_eq!(resolved.bytes, b"pdf bytes");
}

#[tokio::test]
async fn map_resolver_resolves_plain_string_references() {
    let resolver = MapResolver::new().with_entry("invoice-42", b"pdf bytes");

    let resolved_attachment = resolver
        .resolve(&AttachmentReference::new("invoice-42"))
        .await
        .expect("known reference resolves");

    assert_eq!(resolved_attachment.bytes, b"pdf bytes");
}

#[tokio::test]
async fn map_resolver_reports_missing_references() {
    let error = MapResolver::new()
        .resolve(&AttachmentReference::new("missing"))
        .await
        .expect_err("unknown reference fails");

    assert_eq!(error.kind, ResolveErrorKind::NotFound);
}

#[tokio::test]
async fn scheme_router_dispatches_colon_and_slash_references() {
    let resolver = MapResolver::new().with_entry("logo", b"image");
    let router = SchemeRouter::new().with_resolver("asset", resolver);

    let colon = router
        .resolve(&AttachmentReference::new("asset:logo"))
        .await
        .expect("colon reference resolves");
    let slashes = router
        .resolve(&AttachmentReference::new("asset://logo"))
        .await
        .expect("slash reference resolves");

    assert_eq!(colon.bytes, b"image");
    assert_eq!(slashes.bytes, b"image");
}

#[tokio::test]
async fn scheme_router_rejects_unregistered_or_unprefixed_references() {
    let router = SchemeRouter::new().with_resolver("asset", MapResolver::new());

    for reference in ["other:key", "plain-key"] {
        let error = router
            .resolve(&AttachmentReference::new(reference))
            .await
            .expect_err("reference must be routed");
        assert_eq!(error.kind, ResolveErrorKind::UnsupportedReference);
    }
}

#[tokio::test]
async fn scheme_router_passes_full_reference_when_configured() {
    let resolver = MapResolver::new().with_entry("https://example.com/logo.png", b"image");
    let router =
        SchemeRouter::new().with_resolver_using("https", resolver, SchemeDispatch::FullReference);

    let resolved_attachment = router
        .resolve(&AttachmentReference::new("https://example.com/logo.png"))
        .await
        .expect("full reference reaches the resolver unchanged");

    assert_eq!(resolved_attachment.bytes, b"image");
}

#[tokio::test]
async fn scheme_router_matches_schemes_case_insensitively() {
    let resolver = MapResolver::new().with_entry("logo", b"image");
    let router = SchemeRouter::new().with_resolver("asset", resolver);

    let resolved_attachment = router
        .resolve(&AttachmentReference::new("ASSET:logo"))
        .await
        .expect("uppercase scheme routes to the lowercase registration");

    assert_eq!(resolved_attachment.bytes, b"image");
}

#[tokio::test]
async fn scheme_router_rejects_invalid_scheme_prefixes() {
    let router = SchemeRouter::new().with_resolver("asset", MapResolver::new());

    for reference in ["not a scheme:value", "1st:value", ":value"] {
        let error = router
            .resolve(&AttachmentReference::new(reference))
            .await
            .expect_err("invalid scheme prefix must not route");
        assert_eq!(error.kind, ResolveErrorKind::UnsupportedReference);
    }
}

#[test]
#[should_panic(expected = "not a valid attachment reference scheme")]
fn scheme_router_panics_on_invalid_scheme_registration() {
    let _ = SchemeRouter::new().with_resolver("not a scheme", MapResolver::new());
}

#[tokio::test]
async fn fallback_resolver_falls_back_on_unsupported_references() {
    let router =
        SchemeRouter::new().with_resolver("asset", MapResolver::new().with_entry("logo", b"image"));
    let resolver =
        FallbackResolver::new(router, MapResolver::new().with_entry("plain-key", b"bytes"));

    let from_primary = resolver
        .resolve(&AttachmentReference::new("asset:logo"))
        .await
        .expect("routed reference uses the primary");
    let from_fallback = resolver
        .resolve(&AttachmentReference::new("plain-key"))
        .await
        .expect("scheme-less reference uses the fallback");

    assert_eq!(from_primary.bytes, b"image");
    assert_eq!(from_fallback.bytes, b"bytes");
}

#[tokio::test]
async fn fallback_resolver_propagates_authoritative_failures() {
    let resolver = FallbackResolver::new(
        MapResolver::new(),
        MapResolver::new().with_entry("missing", b"1"),
    );

    let error = resolver
        .resolve(&AttachmentReference::new("missing"))
        .await
        .expect_err("not-found is authoritative and must not fall back");

    assert_eq!(error.kind, ResolveErrorKind::NotFound);
}

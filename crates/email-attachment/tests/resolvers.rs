use email_attachment::{AttachmentResolver, MapResolver, ResolveErrorKind, SchemeRouter};
use email_message::AttachmentReference;

#[tokio::test]
async fn map_resolver_resolves_plain_string_references() {
    let resolver = MapResolver::new().with_entry("invoice-42", b"pdf bytes");

    let resolved = resolver
        .resolve(&AttachmentReference::new("invoice-42"))
        .await
        .expect("known reference resolves");

    assert_eq!(resolved.bytes, b"pdf bytes");
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

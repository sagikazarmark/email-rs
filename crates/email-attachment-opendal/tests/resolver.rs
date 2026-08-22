use std::sync::Arc;

use email_attachment::{AttachmentResolver, ResolveErrorKind, SchemeRouter};
use email_attachment_opendal::OpendalResolver;
use email_message::AttachmentReference;
use opendal::{
    Buffer, BytesRange, Capability, EntryMode, Error, ErrorKind, Metadata, OperationContext,
    Operator, layers::CapabilityOverrideLayer, raw::*, services::Memory,
};

#[derive(Debug)]
struct ErrorService {
    error_kind: ErrorKind,
    temporary: bool,
}

impl ErrorService {
    fn error(&self) -> Error {
        Error::new(self.error_kind, "injected store failure").with_temporary(self.temporary)
    }
}

impl Service for ErrorService {
    type Reader = ();
    type Writer = ();
    type Lister = ();
    type Deleter = ();
    type Copier = ();

    fn info(&self) -> ServiceInfo {
        ServiceInfo::with_scheme("error")
    }

    fn capability(&self) -> Capability {
        Capability {
            stat: true,
            read: true,
            ..Default::default()
        }
    }

    async fn create_dir(
        &self,
        _: &OperationContext,
        _: &str,
        _: OpCreateDir,
    ) -> opendal::Result<RpCreateDir> {
        Err(unsupported())
    }

    async fn stat(&self, _: &OperationContext, _: &str, _: OpStat) -> opendal::Result<RpStat> {
        Err(self.error())
    }

    fn read(&self, _: &OperationContext, _: &str, _: OpRead) -> opendal::Result<Self::Reader> {
        Err(unsupported())
    }

    fn write(&self, _: &OperationContext, _: &str, _: OpWrite) -> opendal::Result<Self::Writer> {
        Err(unsupported())
    }

    fn delete(&self, _: &OperationContext) -> opendal::Result<Self::Deleter> {
        Err(unsupported())
    }

    fn list(&self, _: &OperationContext, _: &str, _: OpList) -> opendal::Result<Self::Lister> {
        Err(unsupported())
    }

    fn copy(
        &self,
        _: &OperationContext,
        _: &str,
        _: &str,
        _: OpCopy,
        _: OpCopier,
    ) -> opendal::Result<Self::Copier> {
        Err(unsupported())
    }

    async fn rename(
        &self,
        _: &OperationContext,
        _: &str,
        _: &str,
        _: OpRename,
    ) -> opendal::Result<RpRename> {
        Err(unsupported())
    }

    async fn presign(
        &self,
        _: &OperationContext,
        _: &str,
        _: OpPresign,
    ) -> opendal::Result<RpPresign> {
        Err(unsupported())
    }
}

#[derive(Debug)]
struct UnderreportedLengthService {
    body: Buffer,
}

impl Service for UnderreportedLengthService {
    type Reader = UnderreportedLengthReader;
    type Writer = ();
    type Lister = ();
    type Deleter = ();
    type Copier = ();

    fn info(&self) -> ServiceInfo {
        ServiceInfo::with_scheme("underreported-length")
    }

    fn capability(&self) -> Capability {
        Capability {
            stat: true,
            read: true,
            ..Default::default()
        }
    }

    async fn create_dir(
        &self,
        _: &OperationContext,
        _: &str,
        _: OpCreateDir,
    ) -> opendal::Result<RpCreateDir> {
        Err(unsupported())
    }

    async fn stat(&self, _: &OperationContext, _: &str, _: OpStat) -> opendal::Result<RpStat> {
        Ok(RpStat::new(
            Metadata::new(EntryMode::FILE).with_content_length(0),
        ))
    }

    fn read(&self, _: &OperationContext, _: &str, _: OpRead) -> opendal::Result<Self::Reader> {
        Ok(UnderreportedLengthReader {
            body: self.body.clone(),
        })
    }

    fn write(&self, _: &OperationContext, _: &str, _: OpWrite) -> opendal::Result<Self::Writer> {
        Err(unsupported())
    }

    fn delete(&self, _: &OperationContext) -> opendal::Result<Self::Deleter> {
        Err(unsupported())
    }

    fn list(&self, _: &OperationContext, _: &str, _: OpList) -> opendal::Result<Self::Lister> {
        Err(unsupported())
    }

    fn copy(
        &self,
        _: &OperationContext,
        _: &str,
        _: &str,
        _: OpCopy,
        _: OpCopier,
    ) -> opendal::Result<Self::Copier> {
        Err(unsupported())
    }

    async fn rename(
        &self,
        _: &OperationContext,
        _: &str,
        _: &str,
        _: OpRename,
    ) -> opendal::Result<RpRename> {
        Err(unsupported())
    }

    async fn presign(
        &self,
        _: &OperationContext,
        _: &str,
        _: OpPresign,
    ) -> opendal::Result<RpPresign> {
        Err(unsupported())
    }
}

#[derive(Debug)]
struct UnderreportedLengthReader {
    body: Buffer,
}

impl oio::Read for UnderreportedLengthReader {
    async fn open(
        &self,
        range: BytesRange,
    ) -> opendal::Result<(RpRead, Box<dyn oio::ReadStreamDyn>)> {
        Ok((RpRead::default(), Box::new(self.read_range(range)?)))
    }

    async fn read(&self, range: BytesRange) -> opendal::Result<(RpRead, Buffer)> {
        Ok((RpRead::default(), self.read_range(range)?))
    }
}

impl UnderreportedLengthReader {
    fn read_range(&self, range: BytesRange) -> opendal::Result<Buffer> {
        Ok(self.body.slice(range.to_content_range(self.body.len())?))
    }
}

fn unsupported() -> Error {
    Error::new(ErrorKind::Unsupported, "operation is not supported")
}

#[tokio::test]
async fn resolves_a_path_from_the_configured_operator() {
    let operator = Operator::new(Memory::default()).expect("memory operator builds");
    operator
        .write("reports/weekly.pdf", b"pdf bytes".to_vec())
        .await
        .expect("fixture is written");
    let resolver = OpendalResolver::new(operator, 1024);

    let resolved = resolver
        .resolve(&AttachmentReference::new("reports/weekly.pdf"))
        .await
        .expect("stored attachment resolves");

    assert_eq!(resolved.bytes, b"pdf bytes");
}

#[tokio::test]
async fn classifies_a_missing_object_as_not_found() {
    let operator = Operator::new(Memory::default()).expect("memory operator builds");
    let resolver = OpendalResolver::new(operator, 1024);

    let error = resolver
        .resolve(&AttachmentReference::new("missing.pdf"))
        .await
        .expect_err("missing attachment fails");

    assert_eq!(error.kind, ResolveErrorKind::NotFound);
}

#[tokio::test]
async fn rejects_an_object_larger_than_the_limit() {
    let operator = Operator::new(Memory::default()).expect("memory operator builds");
    operator
        .write("large.bin", b"12345".to_vec())
        .await
        .expect("fixture is written");
    let resolver = OpendalResolver::new(operator, 4);

    let error = resolver
        .resolve(&AttachmentReference::new("large.bin"))
        .await
        .expect_err("oversized attachment fails");

    assert_eq!(error.kind, ResolveErrorKind::TooLarge);
}

#[tokio::test]
async fn caps_reads_when_stat_is_unavailable() {
    let operator = Operator::new(Memory::default())
        .expect("memory operator builds")
        .layer(CapabilityOverrideLayer::new(|mut capability| {
            capability.stat = false;
            capability
        }));
    operator
        .write("large.bin", b"12345".to_vec())
        .await
        .expect("fixture is written");
    let resolver = OpendalResolver::new(operator, 4);

    let error = resolver
        .resolve(&AttachmentReference::new("large.bin"))
        .await
        .expect_err("capped read detects oversized attachment");

    assert_eq!(error.kind, ResolveErrorKind::TooLarge);
}

#[tokio::test]
async fn resolves_full_bytes_when_stat_underreports_content_length() {
    let service = UnderreportedLengthService {
        body: Buffer::from("full attachment bytes"),
    };
    let operator = Operator::from_parts(OperationContext::default(), Arc::new(service));
    let resolver = OpendalResolver::new(operator, 1024);

    let resolved = resolver
        .resolve(&AttachmentReference::new("attachment.bin"))
        .await
        .expect("attachment resolves despite stale metadata");

    assert_eq!(resolved.bytes, b"full attachment bytes");
}

#[tokio::test]
async fn rejects_paths_that_can_escape_the_operator_root() {
    let operator = Operator::new(Memory::default()).expect("memory operator builds");
    let resolver = OpendalResolver::new(operator, 1024);

    for path in [
        "../secret",
        " ../secret",
        "reports/../../secret",
        r"..\secret",
        "/absolute",
    ] {
        let error = resolver
            .resolve(&AttachmentReference::new(path))
            .await
            .expect_err("unsafe path fails");

        assert_eq!(error.kind, ResolveErrorKind::UnsupportedReference);
    }
}

#[tokio::test]
async fn classifies_store_errors_by_retry_policy() {
    let cases = [
        (ErrorKind::PermissionDenied, false, ResolveErrorKind::Denied),
        (ErrorKind::RateLimited, false, ResolveErrorKind::Transient),
        (ErrorKind::Unexpected, true, ResolveErrorKind::Transient),
        (
            ErrorKind::Unsupported,
            false,
            ResolveErrorKind::UnsupportedReference,
        ),
        (
            ErrorKind::IsADirectory,
            false,
            ResolveErrorKind::UnsupportedReference,
        ),
        (
            ErrorKind::RangeNotSatisfied,
            false,
            ResolveErrorKind::Internal,
        ),
        (ErrorKind::Unexpected, false, ResolveErrorKind::Internal),
    ];

    for (error_kind, temporary, expected) in cases {
        let service = ErrorService {
            error_kind,
            temporary,
        };
        let operator = Operator::from_parts(OperationContext::default(), Arc::new(service));
        let resolver = OpendalResolver::new(operator, 1024);

        let error = resolver
            .resolve(&AttachmentReference::new("attachment.bin"))
            .await
            .expect_err("failing store returns a classified error");

        assert_eq!(error.kind, expected, "OpenDAL kind: {error_kind:?}");
    }
}

#[tokio::test]
async fn routes_prefixes_to_separate_configured_stores() {
    let images = Operator::new(Memory::default()).expect("image operator builds");
    images
        .write("logo.png", b"logo".to_vec())
        .await
        .expect("image fixture is written");
    let documents = Operator::new(Memory::default()).expect("document operator builds");
    documents
        .write("invoice.pdf", b"invoice".to_vec())
        .await
        .expect("document fixture is written");
    let router = SchemeRouter::new()
        .with_resolver("images", OpendalResolver::new(images, 1024))
        .with_resolver("documents", OpendalResolver::new(documents, 1024));

    let logo = router
        .resolve(&AttachmentReference::new("images://logo.png"))
        .await
        .expect("image path is routed without its prefix");
    let invoice = router
        .resolve(&AttachmentReference::new("documents:invoice.pdf"))
        .await
        .expect("document path is routed without its prefix");

    assert_eq!(logo.bytes, b"logo");
    assert_eq!(invoice.bytes, b"invoice");
}

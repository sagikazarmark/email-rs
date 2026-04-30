use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use email_message::{AddressList, EmailAddress, Mailbox};

fn bench_email_parse(c: &mut Criterion) {
    c.bench_function("email_parse", |b| {
        b.iter(|| {
            let parsed: EmailAddress = black_box("john.q.public@example.com")
                .parse()
                .expect("email should parse");
            black_box(parsed)
        });
    });
}

fn bench_mailbox_parse(c: &mut Criterion) {
    c.bench_function("mailbox_parse", |b| {
        b.iter(|| {
            let parsed: Mailbox = black_box("Mary Smith <mary@x.test>")
                .parse()
                .expect("mailbox should parse");
            black_box(parsed)
        });
    });
}

fn bench_address_list_parse(c: &mut Criterion) {
    c.bench_function("address_list_parse", |b| {
        b.iter(|| {
            let parsed: AddressList = black_box("Mary Smith <mary@x.test>, jdoe@one.test")
                .parse()
                .expect("address list should parse");
            black_box(parsed)
        });
    });
}

criterion_group!(
    benches,
    bench_email_parse,
    bench_mailbox_parse,
    bench_address_list_parse
);
criterion_main!(benches);

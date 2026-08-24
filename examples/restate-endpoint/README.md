# Restate email endpoint example

A self-contained [Docker Compose](https://docs.docker.com/compose/) stack that runs the
[`restate-email-endpoint`](../../crates/restate-email-endpoint) binary (built with **all transports and all
attachment resolver services** via the `transport-all` cargo feature) against real infrastructure:

| Service         | Purpose                                                       | Host address                                       |
| --------------- | ------------------------------------------------------------- | -------------------------------------------------- |
| `restate`       | Restate server (ingress + admin/UI)                           | http://localhost:8080 (ingress), http://localhost:9070 (UI) |
| `restate-email` | The email endpoint, built from this repository's `Dockerfile` | http://localhost:9080                              |
| `mailpit`       | [Mailpit](https://mailpit.axllent.org/) mock SMTP server with a web UI and REST API | http://localhost:8025            |
| `rustfs`        | [RustFS](https://rustfs.com/) S3-compatible object store backing the attachment resolver | http://localhost:9000 (S3), http://localhost:9001 (console) |

The endpoint is configured (see [`restate-email.example.toml`](restate-email.example.toml)) with:

- a `transactional` SMTP transport pointing at Mailpit (`smtp://mailpit:1025`), and
- a `docs` attachment resolver reading `docs:...` references from the `attachments` bucket in RustFS.

The stack initializes itself through [Compose lifecycle hooks](https://docs.docker.com/compose/how-tos/lifecycle/):

- `pre_start` init containers on `restate-email` create the `attachments` bucket and upload a sample
  `hello.txt` (containing `Hello World`) once RustFS is healthy, and
- a `post_start` hook on `restate` registers the endpoint deployment using the `restate` CLI bundled
  in the server image.

## Prerequisites

- Docker with Compose v5.3+ (the stack uses [`pre_start` init containers](https://docs.docker.com/compose/how-tos/init-containers/))

## Usage

### 1. Create the endpoint configuration

Config files are gitignored, so start from the template:

```sh
cp restate-email.example.toml restate-email.toml
```

### 2. Start the stack

```sh
docker compose up -d --build
```

The first run builds the endpoint image from source, which takes a few minutes. Once everything is
up, the init hooks have created the `attachments` bucket, uploaded `hello.txt`, and registered the
deployment — the `Email` service with its `send` handler shows up in the Restate UI at
<http://localhost:9070>.

### 3. Send an email

Either open the `Email` service in the Restate UI at <http://localhost:9070> and invoke the `send`
handler from the playground, or call the ingress directly. The attachment carries a *reference*
(`docs:hello.txt`) that the endpoint resolves from RustFS before handing the message to Mailpit:

```sh
curl http://localhost:8080/Email/send --json '{
  "transport": "transactional",
  "message": {
    "from": {"type": "mailbox", "email": "sender@example.com"},
    "to": [{"type": "mailbox", "email": "recipient@example.com"}],
    "subject": "Hello from email-rs",
    "body": {"type": "text", "text": "Hello from the Restate email endpoint!"},
    "attachments": [
      {
        "filename": "hello.txt",
        "content_type": "text/plain",
        "body": {"type": "reference", "reference": "docs:hello.txt"}
      }
    ]
  }
}'
```

The response reports the delivery, e.g. `{"report":{"provider":"smtp","accepted":["recipient@example.com"], ...}}`.

### 4. Check the delivery in Mailpit

Open the Mailpit UI at <http://localhost:8025> — the message should be there with `hello.txt`
attached. Or use the [Mailpit API](https://mailpit.axllent.org/docs/api-v1/):

```sh
curl http://localhost:8025/api/v1/messages
```

## Uploading more attachments

Reference uploaded objects as `docs:<key>` in messages. Upload through the RustFS console at
<http://localhost:9001> (credentials `rustfsadmin` \/ `rustfsadmin`), or from the host with curl's
built-in AWS request signing:

```sh
echo "More content" > more.txt
curl -sSf --aws-sigv4 aws:amz:us-east-1:s3 --user rustfsadmin:rustfsadmin \
  -X PUT --data-binary @more.txt http://localhost:9000/attachments/more.txt
```

## Cleanup

```sh
docker compose down -v
```

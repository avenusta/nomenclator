# Security

Report a vulnerability privately through GitHub: open the repository's **Security** tab and choose **Report a vulnerability**. Do not open a public issue or pull request.

Include the affected crate and version, the input that triggers the problem, and what happens.

## Scope

- The `nomenclator` crate reads names as strings. It does not access the filesystem or the network.
- `nomenclator-cli` reads folders and files that the user names.
- `nomenclator-grpc` accepts requests on a TCP port or a Unix socket.

The latest release receives fixes.

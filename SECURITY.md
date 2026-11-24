# Security policy

## Reporting a vulnerability

Please report security vulnerabilities privately, through GitHub's
[private vulnerability reporting](https://github.com/feryx-io/fcp/security/advisories/new)
(the repository's **Security** tab → **Report a vulnerability**). Don't open
a public issue, pull request or discussion for a vulnerability.

A useful report says:

- which component is affected (the binary or crate, and the release or commit);
- what an attacker can do, and from where (over SIP, RTP, the API, a browser, the same host);
- how to reproduce it: a configuration, a request or SIP message, a test;
- anything you know about a fix.

The report stays private between you and the maintainers until it is fixed
and published as a security advisory. We'll credit you in the advisory unless
you'd rather not be named.

## The SIP and media stacks

FCP's SIP stack ([siphon-rs](https://github.com/feryx-io/siphon-rs)) and media
stack ([forge-media](https://github.com/feryx-io/forge-media)) are separate
repositories under `external/`. A vulnerability in one of them can be
reported there, or here if you found it through FCP.

## Supported versions

FCP is pre-release software (see the README). Fixes go into `main` and the
next release. Only the latest release is supported.

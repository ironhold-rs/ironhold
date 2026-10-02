# Security policy

Please report security problems privately, so they can be fixed before the details are public.

## Reporting a vulnerability

Don't open a public issue. Instead, use GitHub's private vulnerability reporting: open the repository's Security tab and click "Report a vulnerability".

Please include:

- The affected crate and version (or commit)
- A description of the issue and its impact
- Steps or code to reproduce it

## What to expect

- An acknowledgement within 3 working days
- An initial assessment within 7 days
- A fix and a published advisory (with credit, if you want it) once a patch is released

## Supported versions

Ironhold is in early development. Only the latest release receives security fixes.

## Scope

In scope: anything where Ironhold's defaults or APIs let an app become insecure without the developer explicitly opting out. Examples include escaping that can be bypassed, a missing security header, or a secret that can leak through `Secret<T>`.

Out of scope: vulnerabilities in an app caused by explicit opt-outs such as `raw_unchecked`, and issues in dependencies that are already publicly reported upstream.

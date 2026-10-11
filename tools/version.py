#!/usr/bin/env python3
"""Validate the application version supplied through DEBONAIR_VERSION, and choose the release
the protocol compatibility checks compare against."""
import os
import re
import subprocess
import sys

NUMBER = r"(?:0|[1-9][0-9]*)"
RELEASE = rf"{NUMBER}\.{NUMBER}\.{NUMBER}"
DEVELOPMENT = "0.0.0"


def resolve():
    value = os.environ.get("DEBONAIR_VERSION", DEVELOPMENT)
    if not re.fullmatch(RELEASE, value):
        raise ValueError("DEBONAIR_VERSION must be MAJOR.MINOR.PATCH without a v prefix")
    return value


def parse(value):
    return tuple(int(n) for n in value.split("."))


def series(version):
    """Releases that must stay compatible: before 1.0 a minor version may break compatibility,
    from 1.0 only a major version may."""
    major, minor, _ = version
    return (0, minor) if major == 0 else (major,)


def protocol_base(version, tags):
    """The release tag the protocol checks compare against: the newest earlier release in the
    same series as `version`, or the newest release for a development build. None when there is
    no such release."""
    releases = sorted(
        (parse(t[1:]), t) for t in tags if re.fullmatch(rf"v{RELEASE}", t)
    )
    if version == DEVELOPMENT:
        return releases[-1][1] if releases else None
    current = parse(version)
    earlier = [t for v, t in releases if v < current and series(v) == series(current)]
    return earlier[-1] if earlier else None


def released_tags():
    """Every release tag in the repository, on any branch. `protocol_base` skips the release being
    built by its version."""
    out = subprocess.run(
        ["git", "tag", "--list", "v*"],
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    return out.split()


def shallow():
    out = subprocess.run(
        ["git", "rev-parse", "--is-shallow-repository"],
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    return out.strip() == "true"


def automatic_base(version):
    """The protocol base for `version` from the repository's release tags. A shallow clone may
    lack the tags, so it finding none is an error rather than a reason to skip the comparison."""
    base = protocol_base(version, released_tags())
    if base is None and shallow():
        raise ValueError(
            "This shallow clone has no release to compare the protocol against. Fetch the "
            "release tags with `git fetch --tags --unshallow`, or set PROTOCOL_BASE."
        )
    return base


if __name__ == "__main__":
    try:
        version = resolve()
        if sys.argv[1:] == ["--protocol-base"]:
            print(automatic_base(version) or "")
        else:
            print(version)
    except (ValueError, subprocess.CalledProcessError) as error:
        sys.exit(str(error))

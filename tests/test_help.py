#!/usr/bin/env python3

import conftest


# this test is also useful to pre-compile vmsh in ci
def test_help(helpers: conftest.Helpers) -> None:
    helpers.run_vmsh_command(["--help"])

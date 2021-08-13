#!/usr/bin/env python3

from pathlib import Path

TEST_ROOT = Path(__file__).parent.resolve()
PROJECT_ROOT = TEST_ROOT.parent
MEASURE_RESULTS = TEST_ROOT.joinpath("measurements")

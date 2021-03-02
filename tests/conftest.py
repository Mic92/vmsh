#!/usr/bin/env python3

import pytest
import sys
from pathlib import Path
from typing import Type


TEST_ROOT = Path(__file__).parent.resolve()
sys.path.append(str(TEST_ROOT.parent))


class Helpers:
    @staticmethod
    def root() -> Path:
        return TEST_ROOT


@pytest.fixture
def helpers() -> Type[Helpers]:
    return Helpers

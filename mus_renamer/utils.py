#!/usr/bin/env python2
# _*_ coding: utf-8 _*_

import logging
import os
import sys

from itertools import groupby
import itertools

from typing import Generator


from pathlib import Path


def alternative_names(filename: Path) -> Generator[str, None, None]:
    yield filename.name
    # if os.path.isfile(filename):
    if filename.is_file():
        yield f"{filename.stem}(Duplicate){filename.suffix}"

        for i in itertools.count(1):
            yield f"{filename.stem}(Duplicate {i}){filename.suffix}"
    else:
        yield f"{filename.name}(Duplicate)"
        for i in itertools.count(1):
            yield f"{filename.name}(Duplicate {i})"


from pathlib import Path


def get_next_name(name: Path) -> Path:
    for alt_name in alternative_names(name):

        full_path = name.parent / alt_name

        if not full_path.exists():
            return full_path


def setup_logging(logger):

    logger.setLevel(logging.DEBUG)
    ch = logging.StreamHandler()
    ch.setLevel(logging.DEBUG)
    formatter = logging.Formatter("%(asctime)s - %(name)s - %(levelname)s - %(message)s")

    ch.setFormatter(formatter)

    logger.addHandler(ch)


def sort_folders(dir):
    # Actually it does not the way it works.
    # Sort order is determined by fat.
    # Use fatsort tool
    dirs = os.listdir(dir)
    for d in dirs:
        print(d)

    dirs.sort()
    for i, d in enumerate(dirs):
        os.utime(d, ((1 + i) * 200, (1 + i) * 200))


def query_yes_no(question, default="yes"):
    """Ask a yes/no question via raw_input() and return their answer.

    "question" is a string that is presented to the user.
    "default" is the presumed answer if the user just hits <Enter>.
        It must be "yes" (the default), "no" or None (meaning
        an answer is required of the user).

    The "answer" return value is True for "yes" or False for "no".
    """
    valid = {"yes": True, "y": True, "ye": True, "no": False, "n": False}
    if default is None:
        prompt = " [y/n] "
    elif default == "yes":
        prompt = " [Y/n] "
    elif default == "no":
        prompt = " [y/N] "
    else:
        raise ValueError("invalid default answer: '%s'" % default)

    while True:
        sys.stdout.write(question + prompt)
        choice = input().lower()
        if default is not None and choice == "":
            return valid[default]
        elif choice in valid:
            return valid[choice]
        else:
            sys.stdout.write("Please respond with 'yes' or 'no' " "(or 'y' or 'n').\n")


# Usage example
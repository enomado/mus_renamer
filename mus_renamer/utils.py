#!/usr/bin/env python2
# _*_ coding: utf-8 _*_

import logging
import os
import sys

from itertools import groupby
import itertools

def alternative_names(filename):
    yield filename
    if os.path.isfile(filename):
        base, ext = os.path.splitext(filename)
        yield "{}(Duplicate){}".format(base, ext)
        for i in itertools.count(1):
            yield "{}(Duplicate {}){ext}".format(base, i, ext)
    else:
        yield "{}(Duplicate)".format(filename)
        for i in itertools.count(1):
            yield "{}(Duplicate {})".format(filename, i)


def get_next_name(name):
    for alt_name in alternative_names(name):
        if not os.path.lexists(alt_name):
            return alt_name


def setup_logging(logger): 
    
    logger.setLevel(logging.DEBUG)
    ch = logging.StreamHandler()
    ch.setLevel(logging.DEBUG)
    formatter = logging.Formatter('%(asctime)s - %(name)s - %(levelname)s - %(message)s')
    
    ch.setFormatter(formatter)

    logger.addHandler(ch)


def sort_folders(dir):
    # Actually it does not the way it works. 
    # Sort order is determined by fat. 
    # Use fatsort tool
    dirs = os.listdir(dir)
    for d in dirs:
        print d
    
    dirs.sort()
    for i, d in enumerate(dirs):
        os.utime(d, ((1+i)*200,(1+i)*200))



def query_yes_no(question, default="yes"):
    """Ask a yes/no question via raw_input() and return their answer.

    "question" is a string that is presented to the user.
    "default" is the presumed answer if the user just hits <Enter>.
        It must be "yes" (the default), "no" or None (meaning
        an answer is required of the user).

    The "answer" return value is True for "yes" or False for "no".
    """
    valid = {"yes": True, "y": True, "ye": True,
             "no": False, "n": False}
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
        choice = raw_input().lower()
        if default is not None and choice == '':
            return valid[default]
        elif choice in valid:
            return valid[choice]
        else:
            sys.stdout.write("Please respond with 'yes' or 'no' "
                             "(or 'y' or 'n').\n")
# Usage example
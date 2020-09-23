#!/usr/bin/env python2
# _*_ coding: utf-8 _*_

import itertools
import argparse
import sys
import re
import shutil
import os, glob
import os.path

import logging
from os import path
from collections import Counter

from pprint import pprint

import mutagen
import transliterate

from itertools import groupby

from mus_renamer.utils import setup_logging, get_next_name, query_yes_no


from pathlib import Path

# http://stackoverflow.com/questions/6309587/call-up-an-editor-vim-from-a-python-script
# sudo mount /dev/sdb1 /mnt/1 -o rw,umask=002,codepage=866,iocharset=utf8,gid=100


logger = logging.getLogger("music")


def is_mp3(filename: str):
    _, ext = os.path.splitext(filename)
    return ext.lower() == ".mp3"


def list_mp3(dirname: Path):
    files = os.listdir(dirname)
    for filename in files:
        abs_filename = dirname / filename  # todo normpath (can be . or ~)
        if is_mp3(str(abs_filename)):
            print(abs_filename)
            yield abs_filename


def clean_garbage(root: Path):
    for (dirpath, dirnames, filenames) in os.walk(root):  # todo
        for filename in filenames:
            if not is_mp3(filename):
                filename = os.path.join(dirpath, filename)
                print(filename)
                os.remove(filename)


def clean_folders(root: Path):
    for (dirpath, dirnames, filenames) in os.walk(root):  # todo
        if not filenames and not dirnames:
            print(dirpath)
            os.removedirs(dirpath)


def sort_by_folders(renames):
    """to deal with nested folders"""

    def sort_key(item):
        old_name, new_name, type_, _, _ = item

        s = list(old_name.parents)
        return len(s)

    sorted_renames = sorted(renames, key=sort_key, reverse=True)
    return sorted_renames


import eyed3

# import id3reader

import chardet
from mutagen.easyid3 import EasyID3
from mutagen.id3 import ID3


def decide(dir):
    """
    returns list of artists and albums in folder
    """
    artists, albums = Counter(), Counter()

    for filename in list_mp3(dir):
        # audio = mutagen.File(filename)
        # audio = ID3(filename, v2_version=3)
        audiofile = eyed3.load(filename)

        # audiofile = id3reader.Reader(filename)
        try:
            # songtitle = audiofile.tag.title
            artist = audiofile.tag.artist
            album = audiofile.tag.album
            # assert len(album) == 1
            # assert len(artist) == 1

            # import IPython

            # IPython.embed()

            # album = audio["album"][0]
            # artist = audio["artist"][0]
            # title = audio["title"][0]

            # import IPython

            # IPython.embed()
            # artist = audio["title"]

            print("bla bla", artist, album)

            # result = chardet.detect(rawdata)

        except KeyError as ex:
            logger.error("no id3 in %s %s", ex, filename)
        else:
            artists.update([artist])
            albums.update([album])

    return artists, albums


def analyze(root: Path):

    data = []

    for (dirpath_raw, _, _) in os.walk(root):  # todo

        dirpath = Path(dirpath_raw).resolve()

        if dirpath == root:
            print(f"ignoring root {root}")
            continue

        artists, albums = decide(dirpath)

        artists = list(artists.keys())
        albums = list(albums.keys())

        if len(artists) == 1 and len(albums) == 1:

            # import IPython
            # IPython.embed()

            new_name = f"{artists[0]} - {albums[0]}"
            data.append((dirpath, new_name, "album", artists[0], albums[0]))
        elif len(artists) == 1 and len(albums) > 1:
            new_name = "{0}".format(artists[0])
            data.append((dirpath, new_name, "collection", artists[0], albums[0]))
        elif len(artists) > 1 and len(albums) == 1:
            new_name = "VA {0}".format(albums[0])
            data.append((dirpath, new_name, "VA", artists[0], albums[0]))
        else:
            data.append((dirpath, dirpath, "folder", None, None))

    return data


def move_to_root(root: Path, renames, args):
    renames = sort_by_folders(renames)

    print(" * " * 3)

    for current_rename in renames:

        old, new, type_, _, _ = current_rename

        print(current_rename)

        if type_ == "folder":
            continue

        new = translit(new)
        # new = new.encode("utf8", "ignore")

        new = root / new

        if os.path.abspath(old) == os.path.abspath(new):
            continue

        target_name = get_next_name(new)

        print("moving", old, "\t", new)
        try:
            if not args.dry_run:
                print("dfdfd", str(old), str(target_name))
                # shutil.move(str(old), str(target_name))
                os.rename(str(old), str(target_name))
                # old.rename(target_name)
            else:
                print(old, target_name)
            # os.rename(old,  target_name)
        except OSError as ex:
            print("'{}' - '{}'".format(repr(old), repr(new)))
            # import IPython; IPython.embed()
            raise


def translit(new):
    lang = transliterate.detect_language(new)
    if lang:
        new = transliterate.translit(new, reversed=True)
    new = cleaned_up_filename = re.sub(r"[\/\\\:\*\?\"\<\>\|]", "", new)
    new = new.strip()
    return new


# TODO: only if more than 2
# refactor: unify coping process


def move_to_artist_folder(root, renames, args):

    new_renames = []
    y = filter(lambda x: x[2] == "album", renames)
    t = sorted(y, key=lambda x: (x[3], x[4]))
    q = groupby(t, key=lambda x: x[3])

    for key, cor in q:
        key_t = translit(key)
        artist_folder = os.path.join(root, key_t)
        for old, new, type_, artist, album in cor:
            album = translit(album)
            new_path = os.path.join(artist_folder, album)
            new_path = new_path.encode("utf8", "ignore")
            new_renames.append((old, new_path))

    renames = sort_by_folders(renames)

    for old, new in new_renames:

        if os.path.abspath(old) == os.path.abspath(new):
            print("okay")
            continue

        target_name = get_next_name(new)

        print(old, "\t", new)
        try:
            if not args.dry_run:
                shutil.move(old, target_name)
            else:
                print(old, target_name)
            # os.rename(old,  target_name)
        except OSError as ex:
            print("'{}' - '{}'".format(repr(old), repr(new)))
            # import IPython; IPython.embed()
            raise


from pprint import pprint


def run(argv=sys.argv):
    parser = argparse.ArgumentParser(description="Arrange music folders by artist/album")

    parser.add_argument("folder", help="directory to process")

    parser.add_argument("--clean_folders", "-c", action="store_true")
    parser.add_argument("--clean_non_mp3", "-j", action="store_true")
    parser.add_argument("--dry_run", "-n", action="store_true")

    args = parser.parse_args()

    setup_logging(logger)

    abs_folder = Path(args.folder).resolve()

    if not query_yes_no(
        "Are you sure you want it on '{}'? \nThis can destroy your data".format(abs_folder), default="no"
    ):
        print("Bye")
        return

    renames = analyze(abs_folder)

    pprint(renames)
    move_to_root(abs_folder, renames, args)

    if args.clean_folders:
        print("cleaning empty folders...")
        clean_folders(abs_folder)
    if args.clean_non_mp3:
        print("cleaning non mp3...")
        clean_garbage(abs_folder)


if __name__ == "__main__":
    run(sys.argv)
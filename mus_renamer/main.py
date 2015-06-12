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

# http://stackoverflow.com/questions/6309587/call-up-an-editor-vim-from-a-python-script
# sudo mount /dev/sdb1 /mnt/1 -o rw,umask=002,codepage=866,iocharset=utf8,gid=100 


logger = logging.getLogger('music')


def is_mp3(filename): 
    _,  ext = os.path.splitext(filename)
    return ext.lower() == '.mp3'


def list_mp3(dirname): 
    files = os.listdir(dirname)
    for filename in files: 
        abs_filename = path.join(dirname, filename) # todo normpath (can be . or ~)
        if is_mp3(abs_filename): 
            print abs_filename
            yield abs_filename


def clean_garbage(root): 
    for (dirpath, dirnames, filenames) in os.walk(root): # todo
        for filename in filenames: 
            if not is_mp3(filename): 
                filename = os.path.join(dirpath, filename)
                print filename
                os.remove(filename)


def clean_folders(root): 
    for (dirpath, dirnames, filenames) in os.walk(root): # todo
        if not filenames and not dirnames: 
            print dirpath
            os.removedirs(dirpath)
        

def sort_by_folders(renames): 
    """to deal with nested folders"""

    def sort_key(item): 
        old_name, new_name, type_, _, _ = item
        s = old_name.split('/')
        return len(s)

    sorted_renames = sorted(renames, key = sort_key, reverse=True) 
    return sorted_renames


def decide(dir): 
    """
    returns list of artists and albums in folder
    """
    artists, albums = Counter(),  Counter()
    
    for filename in list_mp3(dir): 
        audio = mutagen.File(filename)
        try: 
            songtitle = audio["TIT2"]
            artist    = audio["TPE1"]
            album     = audio["TALB"]
            # assert len(album) == 1
            # assert len(artist) == 1

        except KeyError as ex: 
            logger.error('no id3 in %s %s', ex, filename)
        else: 
            artists.update([artist.text[0]])
            albums.update([album.text[0]])
        
    return artists,  albums


def analyze(root):

    data = []
        
    for (dirpath, _, _) in os.walk(root): # todo
        if dirpath == root: 
            print 'ignoring root {0}', format(root)
            continue

        artists, albums = decide(dirpath)
        
        if len(artists) == 1 and len(albums) == 1:
            new_name = u'{0} - {1}'.format(artists.keys()[0], albums.keys()[0])
            data.append((dirpath, new_name, 'album', artists.keys()[0], albums.keys()[0]))
        elif len(artists) == 1 and len(albums) > 1 : 
            new_name = u'{0}'.format(artists.keys()[0])
            data.append((dirpath, new_name, 'collection', artists.keys()[0], albums.keys()[0]))
        elif len(artists) > 1 and len(albums) == 1: 
            new_name = u'VA {0}'.format(albums.keys()[0])
            data.append((dirpath, new_name, 'VA', artists.keys()[0], albums.keys()[0]))
        else:
            data.append((dirpath, dirpath, 'folder', None, None))

    return data


def move_to_root(root, renames, args):
    renames = sort_by_folders(renames)
    
    print ' * ' * 3
        
    for old, new, type_, _, _ in renames: 
        if type_ == 'folder':
            continue

        new = translit(new)
        new = new.encode('utf8', 'ignore')

        new = os.path.join(root, new)

        if  os.path.abspath(old) == os.path.abspath(new): 
            continue

        target_name = get_next_name(new)
        
        print old, '\t', new
        try:
            if not args.dry_run:
                shutil.move(old, target_name)
            else:
                print old, target_name
            # os.rename(old,  target_name)
        except OSError as ex: 
            print u"'{}' - '{}'".format(repr(old), repr(new))
            # import IPython; IPython.embed()
            raise


def translit(new):
    lang = transliterate.detect_language(new)
    if lang:
        new = transliterate.translit(new, reversed = True)
    new = cleaned_up_filename = re.sub(r"[\/\\\:\*\?\"\<\>\|]", "", new)
    new = new.strip()
    return new


# TODO: only if more than 2
# refactor: unify coping process

def move_to_artist_folder(root, renames, args):

    new_renames = []
    y = filter(lambda x: x[2] == 'album', renames)
    t = sorted(y, key = lambda x: (x[3], x[4]))
    q = groupby(t, key = lambda x: x[3])

    for key, cor in q:
        key_t = translit(key)
        artist_folder = os.path.join(root, key_t)
        for old, new, type_, artist, album in cor:
            album = translit(album)
            new_path = os.path.join(artist_folder, album)
            new_path = new_path.encode('utf8', 'ignore')
            new_renames.append((old, new_path))

    renames = sort_by_folders(renames)

    for old, new in new_renames: 

        if  os.path.abspath(old) == os.path.abspath(new): 
            print 'okay'
            continue

        target_name = get_next_name(new)
        
        print old, '\t', new
        try:
            if not args.dry_run:
                shutil.move(old, target_name)
            else:
                print old, target_name
            # os.rename(old,  target_name)
        except OSError as ex: 
            print u"'{}' - '{}'".format(repr(old), repr(new))
            # import IPython; IPython.embed()
            raise


def run(argv = sys.argv): 
    parser = argparse.ArgumentParser(description='Arrange music folders by artist/album')
    
    parser.add_argument('folder', help='directory to process')

    parser.add_argument('--clean_folders', '-c', action='store_true')
    parser.add_argument('--clean_non_mp3', '-j', action='store_true')
    parser.add_argument('--dry_run', '-n', action='store_true')

    args = parser.parse_args()

    setup_logging(logger)

    folder = os.path.normpath(args.folder)
    abs_folder = os.path.abspath(folder)

    if not query_yes_no("Are you sure you want it on '{}'? \nThis can destroy your data".format(abs_folder), default="no"):
        print 'Bye'
        return 

    renames = analyze(folder)
    move_to_root(folder, renames, args)

    if args.clean_folders:
        print 'cleaning empty folders...'
        clean_folders(folder)
    if args.clean_non_mp3:
        print 'cleaning non mp3...'
        clean_garbage(folder)


if __name__ == "__main__":
    run(sys.argv)
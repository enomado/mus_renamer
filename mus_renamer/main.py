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


def clean_shit(root): 
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
           

def setup_logging(): 
    
    logger.setLevel(logging.DEBUG)
    ch = logging.StreamHandler()
    ch.setLevel(logging.DEBUG)
    formatter = logging.Formatter('%(asctime)s - %(name)s - %(levelname)s - %(message)s')
    
    ch.setFormatter(formatter)

    logger.addHandler(ch)


def sort_by_folders(renames): 
    def sort_key(item): 
        old_name, new_name, type_, _, _ = item
        s = old_name.split('/')
        return len(s)

    sorted_renames = sorted(renames, key = sort_key, reverse=True) # to deal with nested folders
    return sorted_renames


# def sort_folders(dir):
#     dirs = os.listdir(dir)
#     for d in dirs:
#         print d
    
#     dirs.sort()
#     for i, d in enumerate(dirs):
#         os.utime(d, ((1+i)*200,(1+i)*200))



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


def move_to_root(root, renames):
    renames = sort_by_folders(renames)
    
    print ' * ' * 3
        
    for old, new, type_, _, _ in renames: 
        if type_ == 'folder':
            continue
        lang = transliterate.detect_language(new)
        if lang:
            new = transliterate.translit(new, reversed = True)

        new =  cleaned_up_filename = re.sub(r"[\/\\\:\*\?\"\<\>\|]", "", new)
        new = new.encode('utf8', 'ignore')

        new = os.path.join(root, new)
        new = new.strip()

        if  os.path.abspath(old) == os.path.abspath(new): 
            print 'okay'
            continue

        target_name = get_next_name(new)
        
        print old, '\t', new
        try:
            shutil.move(old, target_name)
            # os.rename(old,  target_name)
        except OSError as ex: 
            print u"'{}' - '{}'".format(repr(old), repr(new))
            import IPython; IPython.embed()
            raise


def translit(new):
    lang = transliterate.detect_language(new)
    if lang:
        new = transliterate.translit(new, reversed = True)
    new = cleaned_up_filename = re.sub(r"[\/\\\:\*\?\"\<\>\|]", "", new)
    new = new.strip()
    return new


# TODO: only if mode than 2
# refactor: unify coping process

def move_to_artist_folder(root, renames):

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
            shutil.move(old, target_name)
            # os.rename(old,  target_name)
        except OSError as ex: 
            print u"'{}' - '{}'".format(repr(old), repr(new))
            import IPython; IPython.embed()
            raise



def main(root):
    renames = analyze(root)
    # move_to_root(root, renames)
    move_to_artist_folder(root, renames)


def run(argv = sys.argv): 

    parser = argparse.ArgumentParser(description='')
    

    parser.add_argument('action', help='action', choices=['decide', 'sort', 'run', 'clean', 'clean_folders'])
    parser.add_argument('source_directory', help='directory to process')

    # parser.add_argument('--all', '-a', dest='accumulate', action='store_const',
    #                     const=sum, default=max, 
    #                     help='sum the integers (default: find the max)')
    
    # http://docs.python.org/dev/library/argparse.html#the-add-argument-method

    args = parser.parse_args()
    
    process(args)


def process(args):
    setup_logging()

    # TODO: are you sure ?

    pwd = os.getcwd()
    source_directory = os.path.normpath(args.source_directory)
    if args.action == 'decide': 
        print decide(source_directory)
    elif args.action == 'run': 
        main(source_directory)
    elif args.action == 'clean': 
        clean_shit(source_directory)
    elif args.action == 'clean_folders': 
        clean_folders(source_directory)


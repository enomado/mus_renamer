

import mutagen
import os, glob

import id3reader

from os import path

for dir in os.listdir('/mnt'):
    dir = path.join('/mnt', dir)
    if not path.isdir(dir):
        continue
    
    mass_name = ''
    album_name = ''

    os.chdir(dir)
    for track_name in glob.glob('*.mp3'):
        track = id3reader.Reader(track_name)

        artist = track.getValue('performer')
        if artist:
            artist = artist.lower()
            mass_name = artist

        album = track.getValue('album')
        if album:
            album_name = album
        
    if mass_name == '':
        print 'why ', dir
    else:
        new_name = '%s - %s' % (mass_name, album_name)
        new_name = path.join('/mnt', new_name)
        print 'okk ',dir, ' ', new_name
        try: 
            os.rename(dir, new_name)
        except Exception as d: 
            print d
        





# Music folders renamer

## Overview
Useful for a car audio, or to order music collections.
Was written mostly for a car audio.

### How it works
Program manipulates on whole folder.
* creates a list of artists and albums for each folder
* if there are one artist and one album the program treat it as whole album
* for all albums it moves it to the root folder and names it as {artist}-{album}.

There is also function ```move_to_artist_folder``` that move each artist in own folder like
```
/artist/
  album1
  album2
```
 but its not tested well for now.

## Help
```
positional arguments:
  folder               directory to process

optional arguments:
  -h, --help           show this help message and exit
  --clean_folders, -c
  --clean_non_mp3, -j
  --dry_run, -n
```

## Setup
```
git clone git@github.com:enomado/mus_renamer.git 
cd mus_renamer 
pip2 isntall -e .
```

## Run
THIS CAN BE DESTRUCTIVE!
```
cd *Your flash drive folder*
mp3rnm .
```

## Some other tools
```sudo mount /dev/sdb1 /mnt/1 -o rw,umask=002,codepage=866,iocharset=utf8,gid=100 ```
You can also use 
```duff -re0  . | xargs -0 rm ```
for deduplication

or ```fatsort /dev/sdX1```
to sort your folders alphabetical

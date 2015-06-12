# Music folders renamer

## Overview
Useful for car audio, or to order music collections.
Was written mostly for car audio

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
You can also use 
```duff -re0  . | xargs -0 rm ```
for deduplication

or ```fatsort /dev/sdX1```
for sort your folders alphabetical

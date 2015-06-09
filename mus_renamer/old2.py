import IPython
from os.path import join, splitext, exists
from os import walk, listdir

from operator import inv, contains

from itertools import imap, ifilter

from functools import partial

import mutagen

from shutil import rmtree


def LongestCommonSubstring(S1, S2):
    M = [[0]*(1+len(S2)) for i in xrange(1+len(S1))]
    longest, x_longest = 0, 0
    for x in xrange(1,1+len(S1)):
        for y in xrange(1,1+len(S2)):
            if S1[x-1] == S2[y-1]:
                M[x][y] = M[x-1][y-1] + 1
                if M[x][y]>longest:
                    longest = M[x][y]
                    x_longest  = x
            else:
                M[x][y] = 0
    return S1[x_longest-longest: x_longest]


def contains_mp3(files):
    extensions = (ext for name, ext in imap(splitext, files))
    if any(imap(partial(contains, ['.mp3']), extensions)):
        return True
    return False


def without_mp3s(path):
    for root, dirs, files in walk(path):
        if not contains_mp3(files):
            if root != path:
                yield root

def delete_empty(path):
    for i in without_mp3s(path):
        print i
        # rmtree(i)


def is_album(path):
    return contains_mp3(listdir(path))


def common_artist(path):
    files = imap(partial(join, path), listdir(path))
    artist = lambda x: x.get('TPE1')

    files = ifilter(None, imap(mutagen.File, files))

    artists = imap(artist, files)
    artists = imap(lambda x: x.text, artists)

    return reduce(LongestCommonSubstring, artists, artists.next())

from shutil import move

# import string
# valid_chars = "-_.() %s%s" % (string.ascii_letters, string.digits)
# filename = "This Is a (valid) - filename%$&$ .txt"
# ''.join(c for c in filename if c in valid_chars)

def slugify(value):
    """
    Normalizes string, converts to lowercase, removes non-alpha characters,
    and converts spaces to hyphens.
    """
    import unicodedata
    value = unicodedata.normalize('NFKD', value).encode('ascii', 'ignore')
    value = unicode(re.sub('[^\w\s-]', '', value).strip().lower())
    re.sub('[-\s]+', '-', value)


def mass_rename(path):
    for p in listdir(path):
        p = join(path, p)
        if is_album(p):
            artist = common_artist(p)
            if artist != [] and artist[0]:
                nartist = artist[0].encode('utf-8')

                a = filter(str.isalnum, nartist)
                a = "".join(a)
                if not a:
                    a = artist[0]
                else:
                    a = a.decode()
                print a
                dst = join(path, a)
                move(p, dst)


IPython.embed()


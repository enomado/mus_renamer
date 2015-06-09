import os
from setuptools import setup, find_packages

here = os.path.abspath(os.path.dirname(__file__))
README = open(os.path.join(here, 'readme.md')).read()

requires = [
    'mutagen',
    'ipython',
]

setup(name='mus_renamer',
      version='0.0.1',
      description='mus_renamer',
      long_description=README, 
      classifiers=[
        "Programming Language :: Python",
        ],
      author='',
      author_email='',
      url='',
      keywords='',
      packages=find_packages(),
      include_package_data=True,
      zip_safe=False,
      install_requires=requires,
      entry_points="""\
      [console_scripts]
        mp3rnm = mus_renamer.main:run
      """,
      )

#!/usr/bin/env python3

import os
from os import listdir
from os.path import join
from os.path import isfile, join,exists
import sys


dir_input = "."

image_list = [f for f in listdir(dir_input) if isfile(join(dir_input, f)) and not f.lower().startswith("image")]

existing_list = [f for f in listdir(dir_input) if isfile(join(dir_input, f)) and f.lower().startswith("image") and f.lower().endswith(("jpg","png"))]

print("image {} existing {}".format(len(image_list),len(existing_list)))

image_list.sort()

for i in range(len(image_list)):
    number = i + len(existing_list)+1
    filename, ext = os.path.splitext(image_list[i])
    os.rename(image_list[i],"image{:03d}{}".format(number,ext))

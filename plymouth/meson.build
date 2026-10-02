# SPDX-License-Identifier: GPL-3.0-or-later
project('plymouth-3dboot-plugin', 'c',
  version: '0.1.0',
  license: 'GPL-3.0-or-later',
  meson_version: '>= 1.0',
  # Plymouth's headers (ply_trace) need GNU C; -Wall -Wextra -Werror still apply.
  default_options: ['c_std=gnu11', 'warning_level=2', 'werror=true'])

ply_core = dependency('ply-splash-core')
ply_graphics = dependency('ply-splash-graphics')
p3b = dependency('plymouth-3dboot')

plugin = shared_module('plymouth-3dboot',
  'src/plugin.c',
  name_prefix: '',
  dependencies: [ply_core, ply_graphics, p3b],
  gnu_symbol_visibility: 'hidden',
  install: true,
  install_dir: ply_core.get_variable(pkgconfig: 'pluginsdir'))

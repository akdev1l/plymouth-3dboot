# SPDX-License-Identifier: GPL-3.0-or-later
project('plymouth-3dboot-plugin', 'c',
  version: '1.0.0', # x-release-please-version
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

# Headless harness (see tests/harness.c): run with `meson test`.
fixtures = meson.project_source_root() / '..' / 'tests' / 'fixtures'
harness = executable('harness', 'tests/harness.c',
  dependencies: [ply_core, p3b, dependency('dl')],
  link_args: ['-Wl,--export-dynamic'])
test('harness', harness,
  args: [plugin.full_path(), fixtures / 'n64_logo' / 'n64_logo_spin.dae'],
  env: {'LD_LIBRARY_PATH': p3b.get_variable(pkgconfig: 'libdir')},
  depends: plugin,
  timeout: 120)

# Demo theme: the spinning N64 logo.
themedir = ply_core.get_variable(pkgconfig: 'themesdir') / '3dboot-n64'
configure_file(
  input: 'theme/3dboot-n64.plymouth.in',
  output: '3dboot-n64.plymouth',
  configuration: {'THEMEDIR': themedir},
  install: true,
  install_dir: themedir)
install_data(
  fixtures / 'n64_logo' / 'n64_logo_spin.dae',
  fixtures / 'n64_logo' / 'Readme.txt',
  install_dir: themedir)

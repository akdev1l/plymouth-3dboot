# SPDX-License-Identifier: GPL-3.0-or-later
#
# The C library, the Plymouth splash plugin and the demo theme.
#
# Build with `DEV_CONTAINER=fedora scripts/dev.sh just rpm` (see
# scripts/build-rpm.sh), which creates both sources and writes the packages
# to dist/rpm/:
#   Source0  the working tree's files (tracked and untracked, not ignored)
#   Source1  the crates the build needs (cargo-vendor-filterer, Linux only),
#            so the build itself runs offline

# Set by build-rpm.sh to <UTC time>.git<commit>, so each build sorts after
# the previous one and replaces it on upgrade.
%{!?snapshot:%global snapshot 0}

# The meson project lives in plymouth/.
%global _vpath_srcdir plymouth

Name:           plymouth-3dboot
Version:        0.1.0
Release:        0.%{snapshot}%{?dist}
Summary:        CPU rasterizer for animated 3D models, with a C API

# The library: GPL-3.0-or-later, statically linking Rust crates under
# (MIT OR Apache-2.0), MIT, (0BSD OR MIT OR Apache-2.0),
# (MIT OR Zlib OR Apache-2.0), (Zlib OR Apache-2.0 OR MIT) and
# (Unlicense OR MIT); see LICENSE.dependencies in the package.
License:        GPL-3.0-or-later AND MIT AND (MIT OR Apache-2.0) AND (0BSD OR MIT OR Apache-2.0) AND (MIT OR Zlib OR Apache-2.0) AND (Zlib OR Apache-2.0 OR MIT) AND (Unlicense OR MIT)
URL:            https://github.com/akdev/plymouth-3dboot
Source0:        %{name}-%{version}.tar.gz
Source1:        %{name}-%{version}-vendor.tar.gz

ExclusiveArch:  %{rust_arches}

BuildRequires:  cargo >= 1.98
BuildRequires:  cargo-c
BuildRequires:  cargo-rpm-macros
BuildRequires:  gcc
BuildRequires:  meson
BuildRequires:  plymouth-devel

%description
plymouth-3dboot renders animated 3D models (Wavefront OBJ and COLLADA) on the
CPU. This package contains its C library, libplymouth_3dboot.

%package        devel
Summary:        Development files for %{name}
Requires:       %{name}%{?_isa} = %{version}-%{release}

%description    devel
The header and pkg-config file for building against libplymouth_3dboot.

%package -n     plymouth-plugin-3dboot
Summary:        Plymouth splash plugin rendering an animated 3D model
License:        GPL-3.0-or-later
Requires:       %{name}%{?_isa} = %{version}-%{release}
Requires:       plymouth%{?_isa}
# Draws password prompts and messages; without it they stay invisible.
Recommends:     plymouth-plugin-label

%description -n plymouth-plugin-3dboot
A Plymouth splash plugin that shows an animated 3D model, rendered with
libplymouth_3dboot. Themes select the model and the rendering options.

%package -n     plymouth-theme-3dboot-n64
Summary:        Plymouth theme showing the spinning N64 logo
# The theme file is GPL-3.0-or-later. The model is by Shadowth117, whose
# readme (installed with it) says "No credit needed for use"; that is not a
# recognised licence, so this package is for local use only.
License:        GPL-3.0-or-later AND LicenseRef-Shadowth117-no-credit-needed
BuildArch:      noarch
Requires:       plymouth-plugin-3dboot = %{version}-%{release}
# plymouth-set-default-theme
Requires:       plymouth-scripts

%description -n plymouth-theme-3dboot-n64
A Plymouth theme that spins the N64 logo using plymouth-plugin-3dboot. Select
it with: plymouth-set-default-theme 3dboot-n64

%prep
%autosetup -p1 -a1
# Replaces .cargo/config.toml (the repository's only holds wasm settings)
# with Fedora's build profile and the vendored sources.
%cargo_prep -v vendor

%build
%{__cargo} cbuild --locked --profile rpm --target-dir target/capi \
    --library-type cdylib -p plymouth-3dboot-capi \
    --prefix %{_prefix} --libdir %{_libdir} --includedir %{_includedir}
# Build the plugin against the library just built (its -uninstalled.pc).
export PKG_CONFIG_PATH="$(dirname target/capi/*/rpm/plymouth-3dboot.pc)"
# Distribution compiler flags may add warnings that the project's -Werror
# should not turn into build failures.
%meson -Dwerror=false
%meson_build

# Licences and versions of the crates linked into the library (proc macros
# and their dependencies only run at compile time).
set -o pipefail
%{__cargo} tree --locked --offline -p plymouth-3dboot-capi -e normal,no-proc-macro \
    --target "$(rustc --print host-tuple)" --prefix none --format '{p}|{l}' |
    grep -v -e ' (\*)' -e ' (/' | sort -u > crates.txt
cut -d'|' -f2 crates.txt | sort -u > LICENSE.dependencies
# Read by RPM's generator for bundled(crate(...)) Provides.
cut -d'|' -f1 crates.txt > cargo-vendor.txt

%install
%{__cargo} cinstall --locked --profile rpm --target-dir target/capi \
    --library-type cdylib -p plymouth-3dboot-capi \
    --prefix %{_prefix} --libdir %{_libdir} --includedir %{_includedir} \
    --destdir %{buildroot}
%meson_install

%check
# The plugin's headless harness, against the library built above (the build
# directory has no soname link; cinstall creates it on install).
capi_dir="$(dirname target/capi/*/rpm/plymouth-3dboot.pc)"
ln -sf libplymouth_3dboot.so "${capi_dir}/libplymouth_3dboot.so.0"
%meson_test

%files
%license LICENSE LICENSE.dependencies cargo-vendor.txt
%doc README.md
%{_libdir}/libplymouth_3dboot.so.0{,.*}

%files devel
%doc docs/c-api.md
%{_includedir}/plymouth-3dboot.h
%{_libdir}/libplymouth_3dboot.so
%{_libdir}/pkgconfig/plymouth-3dboot.pc

%files -n plymouth-plugin-3dboot
%license LICENSE
%doc docs/plymouth.md
%{_libdir}/plymouth/plymouth-3dboot.so

%files -n plymouth-theme-3dboot-n64
%license LICENSE
%{_datadir}/plymouth/themes/3dboot-n64/

%changelog
* Fri Oct 02 2026 Alex Diaz <alex@akdev.xyz> - 0.1.0-0
- Initial package

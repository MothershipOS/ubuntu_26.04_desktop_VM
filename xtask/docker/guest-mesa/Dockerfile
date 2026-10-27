# mothershipOS: cross-builds Ubuntu's own Mesa package for the Linux guests (amd64 -> arm64). The arm64 libraries
# come from ports.ubuntu.com; the build dependencies are installed per build (they follow the source package).
# Used by `cargo xtask guest-mesa`.
FROM ubuntu:26.04
ENV DEBIAN_FRONTEND=noninteractive
RUN f=/etc/apt/sources.list.d/ubuntu.sources \
 && sed -i 's/^Types: deb$/Types: deb deb-src/; s/^Components:/Architectures: amd64\nComponents:/' $f \
 && printf '\nTypes: deb\nURIs: http://ports.ubuntu.com/ubuntu-ports\nSuites: resolute resolute-updates resolute-security\nComponents: main universe\nArchitectures: arm64\nSigned-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg\n' >> $f \
 && dpkg --add-architecture arm64 \
 && apt-get update -qq \
 && apt-get install -y -qq --no-install-recommends dpkg-dev crossbuild-essential-arm64 debhelper meson ninja-build \
	bison flex glslang-tools wayland-protocols x11proto-dev flatbuffers-compiler \
	python3 python3-yaml python3-mako python3-pycparser python3-setuptools ca-certificates \
 && rm -rf /var/lib/apt/lists/*
# the package indexes for `apt-get source mesa` and the build dependencies
RUN apt-get update -qq

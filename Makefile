# SPDX-License-Identifier: GPL-3.0-or-later
CARGO ?= cargo

.PHONY: all tui desktop server

all: tui desktop server

tui:
	$(CARGO) build --release --locked --bin paper-trail-tui

desktop:
	$(CARGO) build --release --locked --bin paper-trail

server:
	$(CARGO) build --release --locked --bin paper-trail-server

desktop = configure_file(
  input: 'io.github.maennlse.paper-trail.desktop.in',
  output: 'io.github.maennlse.paper-trail.desktop',
  configuration: conf,
)

metainfo = configure_file(
  input: 'io.github.maennlse.paper-trail.metainfo.xml.in',
  output: 'io.github.maennlse.paper-trail.metainfo.xml',
  configuration: conf,
)

install_data(
  desktop,
  install_dir: join_paths(get_option('datadir'), 'applications'),
)

install_data(
  metainfo,
  install_dir: join_paths(get_option('datadir'), 'metainfo'),
)

install_data(
  'icons/io.github.maennlse.paper-trail.svg',
  install_dir: join_paths(get_option('datadir'), 'icons', 'hicolor', 'scalable', 'apps'),
)

meson.add_install_script(
  py,
  files('post_install.py'),
  # The post-install script runs outside the install prefix.  Pass the
  # complete data directory so it updates the cache that GNOME Shell reads.
  join_paths(get_option('prefix'), get_option('datadir')),
)

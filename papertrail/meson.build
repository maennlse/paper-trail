python_sources = files(
  '__init__.py',
  '__main__.py',
  'application.py',
  'folder_colors.py',
  'folder_row.py',
  'note_repository.py',
  'note_row.py',
  'settings.py',
  'window.py',
)

py.install_sources(
  python_sources,
  preserve_path: true,
  subdir: 'papertrail',
)

install_data(
  'ui/window.ui',
  'ui/style.css',
  install_dir: join_paths(py.get_install_dir(), 'papertrail', 'ui'),
)

install_data(
  'styles/builder.xml',
  'styles/builder-dark.xml',
  'styles/peninsula.xml',
  'styles/peninsula-dark.xml',
  install_dir: join_paths(py.get_install_dir(), 'papertrail', 'styles'),
)

configure_file(
  input: 'paper-trail.in',
  output: 'paper-trail',
  configuration: conf,
  install: true,
  install_dir: get_option('bindir'),
  install_mode: 'rwxr-xr-x',
)

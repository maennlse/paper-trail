project(
  'paper-trail',
  version: '0.5.0',
  meson_version: '>=1.1.0',
)

find_program('cargo', required: true)

conf = configuration_data()
conf.set('APP_ID', 'io.github.maennlse.paper-trail')
conf.set('APP_NAME', 'Paper Trail')
conf.set('APP_EXEC', 'paper-trail')
conf.set('APP_VERSION', meson.project_version())

custom_target(
  'paper-trail',
  output: 'paper-trail',
  command: [find_program('sh'), files('build-aux/build-rust.sh'), meson.project_source_root(), '@OUTPUT@', meson.project_build_root()],
  build_always_stale: true,
  build_by_default: true,
  install: true,
  install_dir: get_option('bindir'),
  install_mode: 'rwxr-xr-x',
)
subdir('data')

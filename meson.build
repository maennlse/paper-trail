project(
  'paper-trail',
  version: '0.1.0',
  meson_version: '>=1.1.0',
)

py = import('python').find_installation('python3')

conf = configuration_data()
conf.set('APP_ID', 'io.github.maennlse.paper-trail')
conf.set('APP_NAME', 'Paper Trail')
conf.set('APP_EXEC', 'paper-trail')
conf.set('APP_VERSION', meson.project_version())

subdir('papertrail')
subdir('data')

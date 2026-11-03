# Where logs go. Any value can be overridden by an environment variable:
# LOG_LEVEL (error, warn, info, debug, trace) and LOG_CHANNEL (the default channel).
default = "stderr"
level = "info"

# Write to a file as well by switching the default to a stack:
#
#   default = "stack"
#
#   [channels.stack]
#   driver = "stack"
#   channels = ["stderr", "daily"]
#
#   [channels.daily]
#   driver = "daily"
#   path = "storage/logs/app.log"
#   days = 14

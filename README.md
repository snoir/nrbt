# nrbt

nrbt is a tool to wrap commands and scripts launched via cron. It goals
is to provide an output only if:
* stderr was not empty
* the process exit with a code other than 0

If such cases happen, a simple report will be printed on stdout so cron can
send it at the address configured in the `MAILTO` variable.

This report can also be written to a file (it will be written even if the execution
ended well).

## Example

 # nrbt myscript.sh -o /var/log/myscript.log

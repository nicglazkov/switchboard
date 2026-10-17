#!/bin/sh
# Stands in for ssh in the hub's tests: prints its arguments, the PTY size and TERM, then answers lines until "exit".
echo "ARGS $*"
echo "SIZE $(stty size)"
echo "TERM $TERM"
echo "PID $$"
stty -echo
while IFS= read -r line; do
  case "$line" in
    exit) exit 0 ;;
    exit\ *) exit "${line#exit }" ;;
    size) echo "SIZE $(stty size)" ;;
    flood\ *) head -c "${line#flood }" /dev/zero | tr '\0' 'x'; echo; echo FLOOD-DONE ;;
    swallow\ *) sleep 2; head -c "${line#swallow }" > /dev/null; echo SWALLOWED ;;
    *) echo "GOT $line" ;;
  esac
done

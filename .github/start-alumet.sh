#!/bin/bash
alumet-agent &
PID=$!
sleep 5  # Adjust the sleep time as needed

# Check if the process is still running
if ps -p $PID > /dev/null; then
   kill -SIGINT $PID
   wait $PID
fi
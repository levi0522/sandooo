#!/bin/bash

while true; do
  nohup cargo run >> output.log 2>&1
  sleep 2  # 失败后等待 2 秒再重启，防止死循环过载
done &

#!/bin/bash
source .env
RUN_ID=${1:-85450307-e034-433e-870d-52d1604a9387}
for i in {1..120}; do
  echo "Checking status (attempt $i)..."
  PGPASSWORD=$STORMCHASER_DEV_PASSWORD psql -h localhost -U stormchaser -d stormchaser -t -c "SELECT status FROM workflow_runs WHERE id = '$RUN_ID';" > scripts/status.txt
  # Trim whitespace
  sed -i 's/^[[:space:]]*//;s/[[:space:]]*$//' scripts/status.txt
  read STATUS < scripts/status.txt
  echo "Current Workflow Status: $STATUS"
  if [ "$STATUS" != "running" ] && [ -n "$STATUS" ]; then
    echo "Workflow finished with status: $STATUS"
    PGPASSWORD=$STORMCHASER_DEV_PASSWORD psql -h localhost -U stormchaser -d stormchaser -c "SELECT step_name, status, error, exit_code FROM step_instances WHERE run_id = '$RUN_ID';"
    rm scripts/status.txt
    exit 0
  fi
  sleep 15
done
echo "Timed out waiting for workflow"
PGPASSWORD=$STORMCHASER_DEV_PASSWORD psql -h localhost -U stormchaser -d stormchaser -c "SELECT step_name, status, error, exit_code FROM step_instances WHERE run_id = '$RUN_ID';"
rm scripts/status.txt

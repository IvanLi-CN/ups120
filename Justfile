set shell := ["/bin/sh", "-c"]

# Generic agentd passthrough (release)
agentd +args:
	cd tools/mcu-agentd && cargo run --release -- {{args}}

agentd-start:
	just agentd start

agentd-status:
	just agentd status

agentd-stop:
	just agentd stop

agentd-set-port mcu path="":
	if [ -z "{{path}}" ]; then \
	  cd tools/mcu-agentd && cargo run --release -- set-port {{mcu}}; \
	else \
	  cd tools/mcu-agentd && cargo run --release -- set-port {{mcu}} {{path}}; \
	fi

agentd-get-port mcu:
	cd tools/mcu-agentd && cargo run --release -- get-port {{mcu}}

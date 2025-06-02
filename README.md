# docker-stack-deploy-clean-restart

This is a simple tool for my homelab that I use with [docker-stack-deploy](https://github.com/wez/docker-stack-deploy).

This tool will kill all running docker containers on the host, and then run
docker-stack-deploy.

This tool should rarely be needed, but at times I want to kill all running 
containers on a host and redeploy them with docker-stack-deploy.

A simple bash script would suffice, but what's the fun in that?

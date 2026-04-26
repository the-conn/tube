REGISTRY        ?= quay.io
REPOSITORY      ?= the-conn
IMAGE_NAME      ?= tube
TAG             ?= latest
FULL_IMAGE_NAME := $(REGISTRY)/$(REPOSITORY)/$(IMAGE_NAME):$(TAG)

CONTAINER_ENGINE := $(shell which podman 2>/dev/null || which docker)

.PHONY: all build push test clean help

all: build

## build: Build the multi-stage musl-based container image
build:
	@echo "Building $(FULL_IMAGE_NAME) using $(CONTAINER_ENGINE)..."
	$(CONTAINER_ENGINE) build -t $(FULL_IMAGE_NAME) .

## push: Push the image to the remote registry
push:
	@echo "Pushing $(FULL_IMAGE_NAME) to registry..."
	$(CONTAINER_ENGINE) push $(FULL_IMAGE_NAME)

## test: Run all unit and integration tests
test:
	@echo "Running tests..."
	ENV=TEST cargo test

## clean: Remove local image (standard container cleanup)
clean:
	@echo "Removing local image $(FULL_IMAGE_NAME)..."
	$(CONTAINER_ENGINE) rmi $(FULL_IMAGE_NAME) || true

## help: Show this help message
help:
	@echo "Usage: make [target] [VARIABLES...]"
	@echo ""
	@echo "Targets:"
	@grep -E '^##' $(MAKEFILE_LIST) | sed -e 's/## //' | column -t -s ':'

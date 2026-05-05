# Variables
REGISTRY         ?= quay.io
REPOSITORY       ?= the-conn
IMAGE_NAME       ?= tube
TAG              ?= latest
FULL_IMAGE_NAME  := $(REGISTRY)/$(REPOSITORY)/$(IMAGE_NAME):$(TAG)

CONTAINER_ENGINE := $(shell which podman 2>/dev/null || which docker)

.PHONY: all build image push test lint fmt clean help ci fmt-check

all: fmt lint test build

## build: Compile the binary for the local host architecture
build:
	@echo "Compiling $(IMAGE_NAME) for local host..."
	cargo build --release

## image: Build the multi-stage container image
image:
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

## lint: Run clippy for static analysis
lint:
	@echo "Running clippy..."
	cargo clippy -- -D warnings

## fmt: Check code formatting
fmt:
	@echo "Checking format..."
	cargo +nightly fmt

## fmt-check: Check if fmt is correct
fmt-check:
	@echo "Checking code format..."
	$(CARGO) +nightly fmt --check

## clean: Remove build artifacts and local container images
clean:
	@echo "Cleaning up..."
	cargo clean
	$(CONTAINER_ENGINE) rmi $(FULL_IMAGE_NAME) || true

## help: Show this help message
help:
	@echo "Usage: make [target] [VARIABLES...]"
	@echo ""
	@echo "Targets:"
	@grep -E '^##' $(MAKEFILE_LIST) | sed -e 's/## //' | column -t -s ':'

## ci: Run the ci checks
ci: fmt-check build lint test

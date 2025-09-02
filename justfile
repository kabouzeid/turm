# Cross compilation script

# Variables
binary := "turm"
image := "turm-builder"
container := "turm_extract"
output_dir := "output"

# Build docker image and extract binary to ./output/
dbuild:
    # Build the Docker image
    docker build -t {{image}} .

    # Create a stopped container from the image
    docker create --name {{container}} {{image}}

    # Ensure output directory exists
    mkdir -p {{output_dir}}

    # Copy the compiled binary out of the container into ./output
    docker cp {{container}}:/app/target/release/{{binary}} ./{{output_dir}}/{{binary}}

    # Remove the temporary container
    docker rm {{container}}

    @echo "✅ Binary available at ./{{output_dir}}/{{binary}}"


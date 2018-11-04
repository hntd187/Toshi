pipeline {
    agent {
        docker { image 'toshisearch/builder:latest' }
    }

    stages {
        stage('Build Development') {
            steps {
                sh "cargo build"
            }
        }
        stage('Test') {
            steps {
                sh "cargo test"
            }
        }
        stage('Clippy') {
            steps {
                sh "rustup component add clippy-preview"
                sh "cargo clippy"
            }
        }
        stage('Rustfmt') {
            steps {
                // The build will fail if rustfmt thinks any changes are
                // required.
                sh "cargo fmt --all -- --write-mode diff"
            }
        }
    }
}
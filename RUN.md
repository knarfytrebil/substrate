# Run

## Start a network with 3 nodes

```console
./target/release/substrate --base-path /tmp/subs/alice  --chain=local --alice --node-key 0000000000000000000000000000000000000000000000000000000000000001  --validator --dev

./target/release/substrate --base-path /tmp/subs/bob  --bootnodes /ip4/127.0.0.1/tcp/30333/p2p/QmRpheLN4JWdAnY7HGJfWFNbfkQCb6tFf4vvA6hgjMZKrR --chain=local --bob --port 30334  --validator --dev

./target/release/substrate --base-path /tmp/subs/charlie  --bootnodes /ip4/127.0.0.1/tcp/30333/p2p/QmRpheLN4JWdAnY7HGJfWFNbfkQCb6tFf4vvA6hgjMZKrR --chain=local --charlie --port 30335  --validator --dev
```

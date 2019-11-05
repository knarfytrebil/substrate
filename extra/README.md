Some extra files for configuration.

threshold_enc.rs is a modified example from https://github.com/poanetwork/threshold_crypto/blob/master/examples/threshold_enc.rs, replace it and run to regenerate keys.


 --node-conf-file points to the file with keys. initial_validators in keys should be populated with PeerIds of your nodes, as indicated by "Local node identity is: xxxx" at startup.

-- These must be replaced, since they are individual! --


= Example start:

```
RUST_LOG=info cargo +nightly run --  --dev  --base-path=/tmp/1 --name=node1 --port=9921 --node-conf-file="/store/nodess.json"
RUST_LOG=info cargo +nightly run --  --dev  --base-path=/tmp/2 --name=node2 --port=9922 --node-conf-file="/store/nodess.json" --bootnodes="/ip4/127.0.0.1/tcp/9921/p2p/QmcspA8oQvGgyZQ8PHEzaVgxhd8Bha3mbP8BkrTxxWrpZS"
RUST_LOG=info cargo +nightly run --  --dev  --base-path=/tmp/3 --name=node3 --port=9923 --node-conf-file="/store/nodess.json" --bootnodes="/ip4/127.0.0.1/tcp/9921/p2p/QmcspA8oQvGgyZQ8PHEzaVgxhd8Bha3mbP8BkrTxxWrpZS" --bootnodes="/ip4/127.0.0.1/tcp/9922/p2p/Qmaga7CxmT18pZeaXHKxEc4wsj2cSK8BLhvY4hJ9bdRvAp"
RUST_LOG=info cargo +nightly run --  --dev  --base-path=/tmp/4 --name=node4 --port=9924 --node-conf-file="/store/nodess.json" --bootnodes="/ip4/127.0.0.1/tcp/9921/p2p/QmcspA8oQvGgyZQ8PHEzaVgxhd8Bha3mbP8BkrTxxWrpZS" --bootnodes="/ip4/127.0.0.1/tcp/9922/p2p/Qmaga7CxmT18pZeaXHKxEc4wsj2cSK8BLhvY4hJ9bdRvAp" --bootnodes="/ip4/127.0.0.1/tcp/9923/p2p/QmeihECXEHU4AVGMtLLyTixn9NjJeebLpEygK5RbESUKkR"
```

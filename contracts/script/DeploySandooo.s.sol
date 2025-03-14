// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import "forge-std/Script.sol";
import "../src/Sandooo.sol";

contract DeploySandooo is Script {
    function run() external {
        vm.startBroadcast();
        new Sandooo();
        vm.stopBroadcast();
    }
}

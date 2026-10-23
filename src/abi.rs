//! The contract functions this crate reads. Signatures come from the verified sources
//! (Sourcify, Base 8453) of VotingEscrow, Voter and LeafVoter; gauges and pools are
//! unverified clones, so only their common getters are used.

use alloy_sol_types::sol;

sol! {
    struct Call3 { address target; bool allowFailure; bytes callData; }
    struct Call3Result { bool success; bytes returnData; }
    function aggregate3(Call3[] calls) external payable returns (Call3Result[] returnData);

    interface IVotingEscrow {
        function balanceOf(address owner) external view returns (uint256);
        function tokenOfOwnerByIndex(address owner, uint256 index) external view returns (uint256);
        function ownerOf(uint256 tokenId) external view returns (address);
        function staked(uint256 tokenId) external view returns (uint128 amount, uint48 end, bool isPermanent);
        function balanceOfNFT(uint256 tokenId) external view returns (uint256);
        function delegates(uint256 delegator) external view returns (uint256);
    }

    interface IVoter {
        function tokenStates(uint256 tokenId) external view
            returns (uint128 committed, uint48 lastStakeEnd, uint48 lastAllocated, bool isPermanent);
        function allocationChainIds(uint256 tokenId) external view returns (uint256[] chainIds);
        function allocationChainAmounts(uint256 tokenId, uint256 chainId) external view returns (uint128 allocated);
    }

    interface ILeafVoter {
        function allocationCooldown() external view returns (uint48);
        function maxAccumulatedCooldownReduction() external view returns (uint48);
        function accumulatedCooldownReduction(uint256 tokenId) external view returns (uint48);
        function tokenStates(uint256 tokenId) external view
            returns (address operator, uint48 lastAllocated, bool canVoteForZeroCapGauges, uint128 chainAllocation);
        function allocatedGauges(uint256 tokenId) external view returns (address[] gauges);
        function allocations(uint256 tokenId, address gauge) external view returns (uint128 allocated);
    }

    interface IGauge {
        function pool() external view returns (address);
    }

    interface IPool {
        function token0() external view returns (address);
        function token1() external view returns (address);
        /// Concentrated-liquidity pools only.
        function tickSpacing() external view returns (int24);
        /// Concentrated-liquidity pools: fee in millionths (3000 = 0.3%).
        function fee() external view returns (uint24);
        /// Classic AMM pools only.
        function stable() external view returns (bool);
    }

    interface IERC20 {
        function symbol() external view returns (string);
    }
}

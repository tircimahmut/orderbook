use super::node::{TreeNode, NODES};
use crate::error::ContractResult;
use cosmwasm_std::{Decimal256, Storage, Uint128};
use cw_storage_plus::Map;

pub const TREE: Map<&(u64, i64), u64> = Map::new("tree");

#[allow(dead_code)]
/// Retrieves the root node of a specific book and tick from storage.
pub fn get_root_node(
    storage: &dyn Storage,
    book_id: u64,
    tick_id: i64,
) -> ContractResult<TreeNode> {
    let root_id = TREE.load(storage, &(book_id, tick_id))?;
    Ok(NODES.load(storage, &(book_id, tick_id, root_id))?)
}

#[allow(dead_code)]
/// Calculates the prefix sum of values in the sumtree up to a target ETAS.
pub fn get_prefix_sum(
    storage: &dyn Storage,
    root_node: TreeNode,
    target_etas: Decimal256,
) -> ContractResult<Decimal256> {
    // We start from the root node's sum, which includes everything in the tree.
    // The prefix sum algorithm will chip away at this until we have the correct
    // prefux sum in O(log(N)) time.
    let starting_sum = TreeNode::get_value(&root_node);
    println!("Starting sum: {:?}", starting_sum);

    let result = prefix_sum_walk(storage, &root_node, starting_sum, target_etas);
    println!("Prefix sum result: {:?}", result);
    result
}

fn prefix_sum_walk(
    storage: &dyn Storage,
    node: &TreeNode,
    mut current_sum: Decimal256,
    target_etas: Decimal256,
) -> ContractResult<Decimal256> {
    println!(
        "prefix_sum_walk called with current_sum: {:?}, target_etas: {:?}",
        current_sum, target_etas
    );
    // Sanity check: target ETAS should be inside node's range.
    if target_etas < node.get_min_range() {
        // If the target ETAS is below the root node's range, we can return zero early.
        println!("Target ETAS is below node's range. Returning zero.");
        return Ok(Decimal256::zero());
    } else if target_etas >= node.get_max_range() {
        // If the target ETAS is above the root node's range, we can return the full sum early.
        println!(
            "Target ETAS is above node's range. Returning current_sum: {:?}",
            current_sum
        );
        return Ok(current_sum);
    }

    // If node is a leaf, we just return its full ETAS value. This is because by this point we
    // know the target ETAS is in the node's range, and if the target ETAS is in the range of a
    // leaf, we count the full leaf towards the prefix sum.
    //
    // Recall that the prefix sum is the sum of all the values of all leaves that have a _starting_
    // ETAS below the target ETAS.
    if !node.is_internal() {
        println!("Node is a leaf. Returning current_sum: {:?}", current_sum);
        return Ok(current_sum);
    }

    // --- Attempt walk left ---

    // We fetch both children here since we need to access both regardless of
    // whether we walk left or right.
    let left_child = node.get_left(storage)?;
    println!("Left child: {:?}", left_child);
    let right_child = node.get_right(storage)?;
    println!("Right child: {:?}", right_child);

    // If the left child exists, we run the following logic:
    // * If target ETAS < left child's lower bound, exit early with zero
    // * Else if target ETAS <= upper bound, subtract right child sum from prefix sum and walk left
    //
    // If neither of the above conditions are met, we continue to logic around walking right.
    if let Some(left_child) = left_child {
        if target_etas < left_child.get_min_range() {
            // If the target ETAS is below the left child's range, nothing in the
            // entire tree should be included in the prefix sum, so we return zero.
            //
            // TODO: This should not be possible now that the check above is added.
            // Consider removing or erroring here.
            println!("Target ETAS is below the left child's range. Returning zero.");
            return Ok(Decimal256::zero());
        }

        if target_etas <= left_child.get_max_range() {
            // Since the target ETAS is within the left child's range, we can safely conclude
            // that everything below the right child should not be in our prefix sum.
            let right_sum = right_child.map_or(Decimal256::zero(), |r| r.get_value());
            println!("Right child sum: {:?}", right_sum);

            current_sum = current_sum.checked_sub(right_sum)?;
            println!(
                "Current sum after subtracting right child sum: {:?}",
                current_sum
            );

            // Walk left recursively
            current_sum = prefix_sum_walk(storage, &left_child, current_sum, target_etas)?;
            println!("Current sum after walking left: {:?}", current_sum);

            return Ok(current_sum);
        }
    }

    // --- Attempt walk right ---

    // If right child either doesn't exist, the current prefix sum is simply the sum of the left child,
    // which is fully below the target ETAS, so we return the prefix sum as is.
    if right_child.is_none() {
        println!(
            "Right child does not exist. Returning current_sum: {:?}",
            current_sum
        );
        return Ok(current_sum);
    }

    // In the case where right child exists and the target ETAS is above the left child, we run the following logic:
    // * If target ETAS < right child's lower bound: subtract right child's sum from prefix sum and return
    // * If target ETAS <= right child's upper bound: walk right
    // * If target ETAS > right child's upper bound: return full sum
    let right_child = right_child.unwrap();
    if target_etas < right_child.get_min_range() {
        // If the ETAS is below the right child's range, we know that anything below the right child
        // should not be included in the prefix sum. We subtract the right child's sum from the prefix sum.
        current_sum = current_sum.checked_sub(TreeNode::get_value(&right_child))?;
        println!(
            "Current sum after subtracting right child value: {:?}",
            current_sum
        );
        Ok(current_sum)
    } else if target_etas <= right_child.get_max_range() {
        // If the target ETAS falls in the right child's range, we need to walk right.
        // We do not need to update the prefix sum here because we do not know how much
        // to subtract from it yet. The right walk handles this update.

        // Walk right recursively
        current_sum = prefix_sum_walk(storage, &right_child, current_sum, target_etas)?;
        println!("Current sum after walking right: {:?}", current_sum);

        return Ok(current_sum);
    } else {
        // If we reach here, everything in the tree is below the target ETAS, so we simply return the full sum.
        println!(
            "Target ETAS is above right child's range. Returning current_sum: {:?}",
            current_sum
        );
        return Ok(current_sum);
    }
}

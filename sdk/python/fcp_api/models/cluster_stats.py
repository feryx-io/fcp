from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from typing import cast

if TYPE_CHECKING:
  from ..models.b_tree_map import BTreeMap
  from ..models.cluster_stats_nodes_by_region import ClusterStatsNodesByRegion
  from ..models.cluster_stats_nodes_by_type import ClusterStatsNodesByType





T = TypeVar("T", bound="ClusterStats")



@_attrs_define
class ClusterStats:
    """ Aggregate statistics across all cluster nodes.

        Attributes:
            total_nodes (int):
            healthy_nodes (int):
            degraded_nodes (int):
            offline_nodes (int):
            total_active_calls (int):
            nodes_by_type (ClusterStatsNodesByType):
            nodes_by_region (ClusterStatsNodesByRegion): Nodes per advertised region; nodes without one are not counted
                here.
            nodes_without_region (int): Nodes that advertise no region.
            nodes_with_partial_view (int): Nodes whose own view of the cluster does not look whole — most
                often every node of a region that has lost the others (§2.3).
            versions (BTreeMap):
            version_skew (bool): More than one release is running: an upgrade under way, or one
                that stopped half way.
     """

    total_nodes: int
    healthy_nodes: int
    degraded_nodes: int
    offline_nodes: int
    total_active_calls: int
    nodes_by_type: ClusterStatsNodesByType
    nodes_by_region: ClusterStatsNodesByRegion
    nodes_without_region: int
    nodes_with_partial_view: int
    versions: BTreeMap
    version_skew: bool
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.b_tree_map import BTreeMap # noqa: PLC0415
        from ..models.cluster_stats_nodes_by_region import ClusterStatsNodesByRegion # noqa: PLC0415
        from ..models.cluster_stats_nodes_by_type import ClusterStatsNodesByType # noqa: PLC0415
        total_nodes = self.total_nodes

        healthy_nodes = self.healthy_nodes

        degraded_nodes = self.degraded_nodes

        offline_nodes = self.offline_nodes

        total_active_calls = self.total_active_calls

        nodes_by_type = self.nodes_by_type.to_dict()

        nodes_by_region = self.nodes_by_region.to_dict()

        nodes_without_region = self.nodes_without_region

        nodes_with_partial_view = self.nodes_with_partial_view

        versions = self.versions.to_dict()

        version_skew = self.version_skew


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "total_nodes": total_nodes,
            "healthy_nodes": healthy_nodes,
            "degraded_nodes": degraded_nodes,
            "offline_nodes": offline_nodes,
            "total_active_calls": total_active_calls,
            "nodes_by_type": nodes_by_type,
            "nodes_by_region": nodes_by_region,
            "nodes_without_region": nodes_without_region,
            "nodes_with_partial_view": nodes_with_partial_view,
            "versions": versions,
            "version_skew": version_skew,
        })

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.b_tree_map import BTreeMap # noqa: PLC0415
        from ..models.cluster_stats_nodes_by_region import ClusterStatsNodesByRegion # noqa: PLC0415
        from ..models.cluster_stats_nodes_by_type import ClusterStatsNodesByType # noqa: PLC0415
        d = dict(src_dict)
        total_nodes = d.pop("total_nodes")

        healthy_nodes = d.pop("healthy_nodes")

        degraded_nodes = d.pop("degraded_nodes")

        offline_nodes = d.pop("offline_nodes")

        total_active_calls = d.pop("total_active_calls")

        nodes_by_type = ClusterStatsNodesByType.from_dict(d.pop("nodes_by_type"))




        nodes_by_region = ClusterStatsNodesByRegion.from_dict(d.pop("nodes_by_region"))




        nodes_without_region = d.pop("nodes_without_region")

        nodes_with_partial_view = d.pop("nodes_with_partial_view")

        versions = BTreeMap.from_dict(d.pop("versions"))




        version_skew = d.pop("version_skew")

        cluster_stats = cls(
            total_nodes=total_nodes,
            healthy_nodes=healthy_nodes,
            degraded_nodes=degraded_nodes,
            offline_nodes=offline_nodes,
            total_active_calls=total_active_calls,
            nodes_by_type=nodes_by_type,
            nodes_by_region=nodes_by_region,
            nodes_without_region=nodes_without_region,
            nodes_with_partial_view=nodes_with_partial_view,
            versions=versions,
            version_skew=version_skew,
        )


        cluster_stats.additional_properties = d
        return cluster_stats

    @property
    def additional_keys(self) -> list[str]:
        return list(self.additional_properties.keys())

    def __getitem__(self, key: str) -> Any:
        return self.additional_properties[key]

    def __setitem__(self, key: str, value: Any) -> None:
        self.additional_properties[key] = value

    def __delitem__(self, key: str) -> None:
        del self.additional_properties[key]

    def __contains__(self, key: str) -> bool:
        return key in self.additional_properties

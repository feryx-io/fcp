from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast

if TYPE_CHECKING:
  from ..models.cluster_node_info import ClusterNodeInfo





T = TypeVar("T", bound="ClusterNodesResponse")



@_attrs_define
class ClusterNodesResponse:
    """ Response for GET /api/v1/cluster/nodes

        Attributes:
            available (bool):
            nodes (list[ClusterNodeInfo]):
            message (None | str | Unset):
     """

    available: bool
    nodes: list[ClusterNodeInfo]
    message: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.cluster_node_info import ClusterNodeInfo # noqa: PLC0415
        available = self.available

        nodes = []
        for nodes_item_data in self.nodes:
            nodes_item = nodes_item_data.to_dict()
            nodes.append(nodes_item)



        message: None | str | Unset
        if isinstance(self.message, Unset):
            message = UNSET
        else:
            message = self.message


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "available": available,
            "nodes": nodes,
        })
        if message is not UNSET:
            field_dict["message"] = message

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.cluster_node_info import ClusterNodeInfo # noqa: PLC0415
        d = dict(src_dict)
        available = d.pop("available")

        nodes = []
        _nodes = d.pop("nodes")
        for nodes_item_data in (_nodes):
            nodes_item = ClusterNodeInfo.from_dict(nodes_item_data)



            nodes.append(nodes_item)


        def _parse_message(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        message = _parse_message(d.pop("message", UNSET))


        cluster_nodes_response = cls(
            available=available,
            nodes=nodes,
            message=message,
        )


        cluster_nodes_response.additional_properties = d
        return cluster_nodes_response

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

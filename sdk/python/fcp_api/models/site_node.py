from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset







T = TypeVar("T", bound="SiteNode")



@_attrs_define
class SiteNode:
    """ One node of the site, as its link last reported it.

        Attributes:
            node_id (str):
            node_type (str):
            version (str):
            healthy (bool): Whether the node answered its health check when the link looked.
     """

    node_id: str
    node_type: str
    version: str
    healthy: bool
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        node_id = self.node_id

        node_type = self.node_type

        version = self.version

        healthy = self.healthy


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "node_id": node_id,
            "node_type": node_type,
            "version": version,
            "healthy": healthy,
        })

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        node_id = d.pop("node_id")

        node_type = d.pop("node_type")

        version = d.pop("version")

        healthy = d.pop("healthy")

        site_node = cls(
            node_id=node_id,
            node_type=node_type,
            version=version,
            healthy=healthy,
        )


        site_node.additional_properties = d
        return site_node

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

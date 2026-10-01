from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="CreateFlowRequest")



@_attrs_define
class CreateFlowRequest:
    """ Create flow request model.

        Attributes:
            id (str):
            name (str):
            initial_node (str):
            nodes (Any):
            description (None | str | Unset):
            variables (Any | Unset):
            tenant_id (None | str | Unset):
            enabled (bool | Unset):
     """

    id: str
    name: str
    initial_node: str
    nodes: Any
    description: None | str | Unset = UNSET
    variables: Any | Unset = UNSET
    tenant_id: None | str | Unset = UNSET
    enabled: bool | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        id = self.id

        name = self.name

        initial_node = self.initial_node

        nodes = self.nodes

        description: None | str | Unset
        if isinstance(self.description, Unset):
            description = UNSET
        else:
            description = self.description

        variables = self.variables

        tenant_id: None | str | Unset
        if isinstance(self.tenant_id, Unset):
            tenant_id = UNSET
        else:
            tenant_id = self.tenant_id

        enabled = self.enabled


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "id": id,
            "name": name,
            "initial_node": initial_node,
            "nodes": nodes,
        })
        if description is not UNSET:
            field_dict["description"] = description
        if variables is not UNSET:
            field_dict["variables"] = variables
        if tenant_id is not UNSET:
            field_dict["tenant_id"] = tenant_id
        if enabled is not UNSET:
            field_dict["enabled"] = enabled

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        id = d.pop("id")

        name = d.pop("name")

        initial_node = d.pop("initial_node")

        nodes = d.pop("nodes")

        def _parse_description(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        description = _parse_description(d.pop("description", UNSET))


        variables = d.pop("variables", UNSET)

        def _parse_tenant_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        tenant_id = _parse_tenant_id(d.pop("tenant_id", UNSET))


        enabled = d.pop("enabled", UNSET)

        create_flow_request = cls(
            id=id,
            name=name,
            initial_node=initial_node,
            nodes=nodes,
            description=description,
            variables=variables,
            tenant_id=tenant_id,
            enabled=enabled,
        )


        create_flow_request.additional_properties = d
        return create_flow_request

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

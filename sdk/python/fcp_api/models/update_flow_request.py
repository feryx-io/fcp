from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="UpdateFlowRequest")



@_attrs_define
class UpdateFlowRequest:
    """ Update flow request model.

        Attributes:
            name (None | str | Unset):
            description (None | str | Unset):
            initial_node (None | str | Unset):
            nodes (Any | Unset):
            variables (Any | Unset):
            enabled (bool | None | Unset):
     """

    name: None | str | Unset = UNSET
    description: None | str | Unset = UNSET
    initial_node: None | str | Unset = UNSET
    nodes: Any | Unset = UNSET
    variables: Any | Unset = UNSET
    enabled: bool | None | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        name: None | str | Unset
        if isinstance(self.name, Unset):
            name = UNSET
        else:
            name = self.name

        description: None | str | Unset
        if isinstance(self.description, Unset):
            description = UNSET
        else:
            description = self.description

        initial_node: None | str | Unset
        if isinstance(self.initial_node, Unset):
            initial_node = UNSET
        else:
            initial_node = self.initial_node

        nodes = self.nodes

        variables = self.variables

        enabled: bool | None | Unset
        if isinstance(self.enabled, Unset):
            enabled = UNSET
        else:
            enabled = self.enabled


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
        })
        if name is not UNSET:
            field_dict["name"] = name
        if description is not UNSET:
            field_dict["description"] = description
        if initial_node is not UNSET:
            field_dict["initial_node"] = initial_node
        if nodes is not UNSET:
            field_dict["nodes"] = nodes
        if variables is not UNSET:
            field_dict["variables"] = variables
        if enabled is not UNSET:
            field_dict["enabled"] = enabled

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        def _parse_name(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        name = _parse_name(d.pop("name", UNSET))


        def _parse_description(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        description = _parse_description(d.pop("description", UNSET))


        def _parse_initial_node(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        initial_node = _parse_initial_node(d.pop("initial_node", UNSET))


        nodes = d.pop("nodes", UNSET)

        variables = d.pop("variables", UNSET)

        def _parse_enabled(data: object) -> bool | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(bool | None | Unset, data)

        enabled = _parse_enabled(d.pop("enabled", UNSET))


        update_flow_request = cls(
            name=name,
            description=description,
            initial_node=initial_node,
            nodes=nodes,
            variables=variables,
            enabled=enabled,
        )


        update_flow_request.additional_properties = d
        return update_flow_request

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

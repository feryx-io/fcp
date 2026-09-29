from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset






T = TypeVar("T", bound="NodeCapacity")



@_attrs_define
class NodeCapacity:
    """ What a node will carry, from its `[admission]` section, and what it
    is carrying now (`docs/CAPACITY_AND_SURVIVABILITY.md` §7). Zero means
    no limit, as it does in the configuration.

        Attributes:
            max_concurrent_calls (int | Unset): `[admission] max_concurrent_calls`.
            max_cps (int | Unset): `[admission] max_cps`.
            max_concurrent_registrations (int | Unset): `[admission] max_concurrent_registrations`.
            registrations (int | Unset): Registrations held now.
            cps (int | Unset): Calls started in the last second.
     """

    max_concurrent_calls: int | Unset = UNSET
    max_cps: int | Unset = UNSET
    max_concurrent_registrations: int | Unset = UNSET
    registrations: int | Unset = UNSET
    cps: int | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        max_concurrent_calls = self.max_concurrent_calls

        max_cps = self.max_cps

        max_concurrent_registrations = self.max_concurrent_registrations

        registrations = self.registrations

        cps = self.cps


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
        })
        if max_concurrent_calls is not UNSET:
            field_dict["max_concurrent_calls"] = max_concurrent_calls
        if max_cps is not UNSET:
            field_dict["max_cps"] = max_cps
        if max_concurrent_registrations is not UNSET:
            field_dict["max_concurrent_registrations"] = max_concurrent_registrations
        if registrations is not UNSET:
            field_dict["registrations"] = registrations
        if cps is not UNSET:
            field_dict["cps"] = cps

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        max_concurrent_calls = d.pop("max_concurrent_calls", UNSET)

        max_cps = d.pop("max_cps", UNSET)

        max_concurrent_registrations = d.pop("max_concurrent_registrations", UNSET)

        registrations = d.pop("registrations", UNSET)

        cps = d.pop("cps", UNSET)

        node_capacity = cls(
            max_concurrent_calls=max_concurrent_calls,
            max_cps=max_cps,
            max_concurrent_registrations=max_concurrent_registrations,
            registrations=registrations,
            cps=cps,
        )


        node_capacity.additional_properties = d
        return node_capacity

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

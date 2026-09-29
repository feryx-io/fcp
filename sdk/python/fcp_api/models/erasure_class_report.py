from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="ErasureClassReport")



@_attrs_define
class ErasureClassReport:
    """ 
        Attributes:
            treatment (str): `delete`, `pseudonymise`, `clear_owner` or `keep`.
            items (int): Items it was (or, in a preview, would be) done to.
            held (int | Unset): Items left alone because a legal hold covers them.
            errors (list[str] | Unset): Items it could not act on, with why.
            sample (list[str] | Unset): The first items' ids.
            truncated (bool | Unset):
     """

    treatment: str
    items: int
    held: int | Unset = UNSET
    errors: list[str] | Unset = UNSET
    sample: list[str] | Unset = UNSET
    truncated: bool | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        treatment = self.treatment

        items = self.items

        held = self.held

        errors: list[str] | Unset = UNSET
        if not isinstance(self.errors, Unset):
            errors = self.errors



        sample: list[str] | Unset = UNSET
        if not isinstance(self.sample, Unset):
            sample = self.sample



        truncated = self.truncated


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "treatment": treatment,
            "items": items,
        })
        if held is not UNSET:
            field_dict["held"] = held
        if errors is not UNSET:
            field_dict["errors"] = errors
        if sample is not UNSET:
            field_dict["sample"] = sample
        if truncated is not UNSET:
            field_dict["truncated"] = truncated

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        treatment = d.pop("treatment")

        items = d.pop("items")

        held = d.pop("held", UNSET)

        errors = cast(list[str], d.pop("errors", UNSET))


        sample = cast(list[str], d.pop("sample", UNSET))


        truncated = d.pop("truncated", UNSET)

        erasure_class_report = cls(
            treatment=treatment,
            items=items,
            held=held,
            errors=errors,
            sample=sample,
            truncated=truncated,
        )


        erasure_class_report.additional_properties = d
        return erasure_class_report

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

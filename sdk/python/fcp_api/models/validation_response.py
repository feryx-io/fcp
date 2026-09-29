from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast

if TYPE_CHECKING:
  from ..models.e911_address_response import E911AddressResponse





T = TypeVar("T", bound="ValidationResponse")



@_attrs_define
class ValidationResponse:
    """ 
        Attributes:
            valid (bool):
            normalized (E911AddressResponse):
            issues (list[str]):
            notes (list[str]): What the validator could not do on an otherwise good address: the
                parts of the street it could not tell apart, say.
            validator (str):
            provider_reference (None | str | Unset): The provider's reference for this answer, when it gave one.
     """

    valid: bool
    normalized: E911AddressResponse
    issues: list[str]
    notes: list[str]
    validator: str
    provider_reference: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.e911_address_response import E911AddressResponse # noqa: PLC0415
        valid = self.valid

        normalized = self.normalized.to_dict()

        issues = self.issues



        notes = self.notes



        validator = self.validator

        provider_reference: None | str | Unset
        if isinstance(self.provider_reference, Unset):
            provider_reference = UNSET
        else:
            provider_reference = self.provider_reference


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "valid": valid,
            "normalized": normalized,
            "issues": issues,
            "notes": notes,
            "validator": validator,
        })
        if provider_reference is not UNSET:
            field_dict["provider_reference"] = provider_reference

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.e911_address_response import E911AddressResponse # noqa: PLC0415
        d = dict(src_dict)
        valid = d.pop("valid")

        normalized = E911AddressResponse.from_dict(d.pop("normalized"))




        issues = cast(list[str], d.pop("issues"))


        notes = cast(list[str], d.pop("notes"))


        validator = d.pop("validator")

        def _parse_provider_reference(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        provider_reference = _parse_provider_reference(d.pop("provider_reference", UNSET))


        validation_response = cls(
            valid=valid,
            normalized=normalized,
            issues=issues,
            notes=notes,
            validator=validator,
            provider_reference=provider_reference,
        )


        validation_response.additional_properties = d
        return validation_response

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
